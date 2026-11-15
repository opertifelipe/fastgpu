use crate::{
    config::{REMOTE_HOME, WORKSPACE, atomic_file, atomic_json, lock_file},
    models::{DEPLOY_FILE, MODEL_DIR, preset},
    process,
    ssh::{clean_path, remote_path},
    worker::{Deployment, Verification, hash_file, modified, prepare_model, supervisor},
};
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::Command;

pub fn default_model_name(target: &str) -> String {
    let base = Path::new(target)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let name = base.strip_suffix(".gguf").unwrap_or(&base).to_lowercase();
    let replaced = Regex::new(r"[^a-zA-Z0-9._-]+")
        .unwrap()
        .replace_all(&name, "-");
    let result: String = replaced
        .trim_matches(['.', '-', '_'])
        .chars()
        .take(96)
        .collect();
    if result.is_empty() {
        "model".into()
    } else {
        result
    }
}
pub fn model_path(s: &str) -> Result<String> {
    if !Path::new(s).is_absolute() {
        return remote_path(s);
    }
    if s.contains(['\0', '\r', '\n']) {
        bail!("invalid path");
    }
    let path = clean_path(Path::new(s));
    if path.starts_with(WORKSPACE) || path.starts_with(REMOTE_HOME) {
        return Ok(path.to_string_lossy().into());
    }
    bail!("use a path on the volume: /workspace/... or /home/gpu/...")
}
pub fn validate_deployment(d: &Deployment) -> Result<()> {
    if !["llama.cpp", "ollama", "vllm"].contains(&d.engine.as_str()) {
        bail!("supported engines: llama.cpp, ollama, vllm");
    }
    if !Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9._-]{0,95}$")?.is_match(&d.model) {
        bail!("invalid model name: use letters, numbers, dots, hyphens, or underscores");
    }
    if !(512..=262144).contains(&d.context) {
        bail!("allowed context size: 512..262144 tokens");
    }
    if d.hf
        && (d.engine != "vllm"
            || !Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]*/[A-Za-z0-9][A-Za-z0-9._-]*$")?
                .is_match(&d.target))
    {
        bail!("--hf requires vllm and a Hugging Face organization/model ID");
    }
    if d.target.is_empty()
        || [&d.target, &d.tokenizer, &d.modelfile]
            .iter()
            .any(|s| s.contains(['\0', '\r', '\n']))
    {
        bail!("invalid model or tokenizer path");
    }
    if !d.modelfile.is_empty() && d.engine != "ollama" {
        bail!("--modelfile requires Ollama");
    }
    if !d.tokenizer.is_empty() && d.engine != "vllm" {
        bail!("--tokenizer requires vLLM");
    }
    if d.engine == "vllm"
        && !d.hf
        && d.tokenizer.is_empty()
        && (preset(&d.target).is_some() || d.target.to_lowercase().ends_with(".gguf"))
    {
        bail!("for GGUF with vLLM, specify --tokenizer with a Hugging Face ID or persistent path");
    }
    if d.engine == "ollama" && !d.args.is_empty() {
        bail!("use --modelfile to customize Ollama, not --arg");
    }
    let reserved = [
        "--host",
        "--port",
        "--model",
        "--alias",
        "--served-model-name",
        "--ctx-size",
        "--max-model-len",
        "--tokenizer",
        "--config",
        "--api-key",
        "--ssl",
        "--rpc",
        "--download-dir",
        "--daemon",
        "--help",
        "--version",
    ];
    for arg in &d.args {
        if arg.contains('\0') {
            bail!("invalid argument");
        }
        if arg.starts_with('-') && !arg.starts_with("--") && arg.parse::<f64>().is_err() {
            bail!("use long options with --arg: {arg}");
        }
        let flag = arg.split('=').next().unwrap_or("");
        if flag.starts_with("--ssl-")
            || (flag.starts_with("--") && reserved.iter().any(|r| r.starts_with(flag)))
        {
            bail!("option reserved for the CLI: {flag}");
        }
    }
    Ok(())
}
pub fn persistent_path(root: &Path, name: &Path) -> Result<PathBuf> {
    let base = root.canonicalize()?;
    let path = name.canonicalize()?;
    if !path.starts_with(&base) {
        bail!(
            "{} is not on the Network Volume: save it in /workspace or the persistent home directory",
            name.display()
        );
    }
    Ok(path)
}
pub fn gguf_fingerprint(path: &Path, cache_dir: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() < 24 {
        bail!("invalid GGUF file");
    }
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    if &magic != b"GGUF" {
        bail!("file does not contain a GGUF header");
    }
    let key = format!("{:x}", Sha256::digest(path.as_os_str().as_encoded_bytes()));
    let cache = cache_dir.join(format!("{key}.json"));
    let known: Verification = fs::read(&cache)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if known.size == metadata.len()
        && known.modified == modified(&metadata)
        && known.sha256.len() == 64
    {
        return Ok(known.sha256);
    }
    file.seek(SeekFrom::Start(0))?;
    let hash = hash_file(&mut file)?;
    let after = file.metadata()?;
    if after.len() != metadata.len() || modified(&after) != modified(&metadata) {
        bail!("model changed during verification: finish saving before deployment");
    }
    atomic_json(
        &cache,
        &Verification {
            size: metadata.len(),
            modified: modified(&metadata),
            sha256: hash.clone(),
        },
    )?;
    Ok(hash)
}
pub fn deployment_fingerprint(d: &Deployment) -> Result<String> {
    if d.hf {
        return Ok(format!("hf:{}", d.target));
    }
    let file = persistent_path(Path::new(WORKSPACE), Path::new(&d.target))?;
    let mut sum = if file.is_dir() {
        if d.engine == "llama.cpp" {
            bail!("llama.cpp requires a GGUF file, not a checkpoint");
        }
        fs::metadata(file.join("config.json"))
            .context("incomplete checkpoint: missing config.json")?;
        let mut hash = Sha256::new();
        for entry in walkdir::WalkDir::new(&file)
            .sort_by_file_name()
            .follow_links(false)
        {
            let entry = entry?;
            if entry.file_type().is_dir() {
                continue;
            }
            let actual = persistent_path(Path::new(WORKSPACE), entry.path())?;
            let metadata = fs::metadata(actual)?;
            hash.update(format!(
                "{}:{}:{}\n",
                entry.path().display(),
                metadata.len(),
                modified(&metadata)
            ));
        }
        format!("{:x}", hash.finalize())
    } else {
        let hash = gguf_fingerprint(&file, &Path::new(MODEL_DIR).join(".checks"))?;
        if d.engine == "vllm" && d.tokenizer.is_empty() {
            bail!(
                "for GGUF with vLLM, specify --tokenizer with a Hugging Face ID or persistent path"
            );
        }
        hash
    };
    if !d.modelfile.is_empty() {
        let path = persistent_path(Path::new(WORKSPACE), Path::new(&d.modelfile))?;
        let mut hash = Sha256::new();
        hash.update(sum);
        hash.update(fs::read(path)?);
        sum = format!("{:x}", hash.finalize());
    }
    Ok(sum)
}
pub fn engine_command(d: &Deployment) -> (&'static str, Vec<String>) {
    let context = d.context.to_string();
    let (name, args) = match d.engine.as_str() {
        "ollama" => return ("ollama", vec!["serve".into()]),
        "vllm" => (
            "vllm",
            vec![
                "serve",
                &d.target,
                "--host",
                "127.0.0.1",
                "--port",
                "8080",
                "--served-model-name",
                &d.model,
                "--max-model-len",
                &context,
            ],
        ),
        _ => (
            "llama-server",
            vec![
                "--model",
                &d.target,
                "--alias",
                &d.model,
                "--host",
                "127.0.0.1",
                "--port",
                "8080",
                "--ctx-size",
                &context,
                "--parallel",
                "1",
                "--n-gpu-layers",
                "all",
                "--fit",
                "off",
                "--jinja",
                "--no-mmproj",
            ],
        ),
    };
    let mut args: Vec<String> = args.into_iter().map(str::to_string).collect();
    if d.engine == "vllm" && !d.tokenizer.is_empty() {
        args.extend(["--tokenizer".into(), d.tokenizer.clone()]);
    }
    args.extend(d.args.clone());
    (name, args)
}
pub fn engine_env() -> [(&'static str, &'static str); 2] {
    [
        ("OLLAMA_HOST", "127.0.0.1:8080"),
        ("OLLAMA_MODELS", "/workspace/.gpu/ollama/models"),
    ]
}
async fn get(base: &str, path: &str) -> Result<Value> {
    let response = reqwest::Client::new()
        .get(format!("{base}{path}"))
        .timeout(Duration::from_secs(5))
        .send()
        .await?;
    if response.status().as_u16() != 200 {
        bail!("engine not ready: HTTP {}", response.status().as_u16());
    }
    // A health endpoint may return plain text, while catalogs must contain JSON.
    if path == "/health" {
        return Ok(Value::Null);
    }
    Ok(response.json().await?)
}
pub async fn catalog_model(base: &str, d: &Deployment) -> Result<()> {
    let value = get(base, "/v1/models").await?;
    if value["data"].as_array().is_some_and(|models| {
        models.iter().any(|m| {
            m["id"]
                .as_str()
                .is_some_and(|id| id.strip_suffix(":latest").unwrap_or(id) == d.model)
        })
    }) {
        return Ok(());
    }
    bail!("model {} not ready", d.model)
}
pub async fn ready_model(base: &str, d: &Deployment) -> Result<()> {
    if d.engine != "ollama" {
        get(base, "/health").await?;
    }
    catalog_model(base, d).await?;
    if d.engine != "ollama" {
        return Ok(());
    }
    let value = get(base, "/api/ps").await?;
    if value["models"].as_array().is_some_and(|models| {
        models.iter().any(|m| {
            ["model", "name"].iter().any(|key| {
                m[key]
                    .as_str()
                    .is_some_and(|id| id.strip_suffix(":latest").unwrap_or(id) == d.model)
            })
        })
    }) {
        return Ok(());
    }
    bail!("Ollama model exists but is not loaded")
}
pub async fn worker_deploy(mut wanted: Deployment) -> Result<()> {
    let _lock = lock_file(
        &Path::new(DEPLOY_FILE).with_file_name("lock"),
        "another fastgpu operation is modifying this profile",
    )?;
    validate_deployment(&wanted)?;
    if let Some(model) = preset(&wanted.target) {
        prepare_model(&wanted.target).await?;
        wanted.target = Path::new(MODEL_DIR)
            .join(model.file)
            .to_string_lossy()
            .into();
    }
    if !wanted.hf {
        wanted.target = persistent_path(Path::new(WORKSPACE), Path::new(&wanted.target))?
            .to_string_lossy()
            .into();
    }
    wanted.sha256 = deployment_fingerprint(&wanted)?;
    let previous: Deployment = fs::read(DEPLOY_FILE)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if previous == wanted && ready_model("http://127.0.0.1:8080", &wanted).await.is_ok() {
        println!("Model already ready");
        return Ok(());
    }
    let _ = process::run(
        Command::new("supervisorctl")
            .args([
                "-c",
                "/etc/supervisor/supervisord.conf",
                "stop",
                "inference",
            ])
            .stderr(Stdio::null()),
        None,
        true,
        false,
    )
    .await;
    atomic_json(Path::new(DEPLOY_FILE), &wanted)?;
    supervisor("start").await?;
    if wanted.engine != "ollama" {
        return Ok(());
    }
    tokio::time::timeout(Duration::from_secs(20 * 60), async {
        loop {
            if get("http://127.0.0.1:8080", "/api/version").await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
        let modelfile = if wanted.modelfile.is_empty() {
            let path = "/workspace/.gpu/ollama/Modelfile";
            atomic_file(
                Path::new(path),
                format!(
                    "FROM {}\nPARAMETER num_ctx {}\n",
                    serde_json::to_string(&wanted.target)?,
                    wanted.context
                )
                .as_bytes(),
            )?;
            path
        } else {
            &wanted.modelfile
        };
        if previous != wanted
            || catalog_model("http://127.0.0.1:8080", &wanted)
                .await
                .is_err()
        {
            process::run(
                Command::new("ollama")
                    .args(["create", &wanted.model, "-f", modelfile])
                    .envs(engine_env()),
                None,
                false,
                false,
            )
            .await?;
        }
        let mut response = reqwest::Client::new()
            .post("http://127.0.0.1:8080/api/generate")
            .json(&json!({"model":wanted.model,"prompt":"","stream":false,"keep_alive":-1}))
            .send()
            .await?;
        if response.status().as_u16() != 200 {
            bail!(
                "Ollama loading failed: HTTP {}; check fastgpu logs",
                response.status().as_u16()
            );
        }
        while response.chunk().await?.is_some() {}
        Ok(())
    })
    .await
    .context("timed out loading Ollama model")?
}
pub async fn worker_ready() -> Result<()> {
    ready_model(
        "http://127.0.0.1:8080",
        &serde_json::from_slice(&fs::read(DEPLOY_FILE)?)?,
    )
    .await
}

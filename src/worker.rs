use crate::{
    config::{atomic_json, lock_file},
    engines::{deployment_fingerprint, engine_command, engine_env, validate_deployment},
    models::{DEPLOY_FILE, MODEL_DIR, ModelPreset, preset, valid_model},
    process,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{
        fs::{MetadataExt, OpenOptionsExt},
        process::CommandExt,
    },
    path::Path,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Deployment {
    pub model: String,
    pub engine: String,
    pub target: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hf: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub tokenizer: String,
    #[serde(skip_serializing_if = "Vec::is_empty", deserialize_with = "null_args")]
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub modelfile: String,
    pub context: i64,
    pub sha256: String,
}
fn null_args<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Verification {
    pub size: u64,
    pub modified: i64,
    pub sha256: String,
}
pub fn modified(metadata: &fs::Metadata) -> i64 {
    metadata.mtime() * 1_000_000_000 + metadata.mtime_nsec()
}
pub fn hash_file(file: &mut File) -> Result<String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn verify_model(file: &Path, model: &ModelPreset) -> Result<()> {
    let metadata = fs::metadata(file)?;
    if metadata.len() != model.size {
        bail!(
            "incorrect GGUF size: {}, expected {}",
            metadata.len(),
            model.size
        );
    }
    let cache = format!("{}.verified.json", file.display());
    let cached: Verification = fs::read(&cache)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    if cached.size == metadata.len()
        && cached.modified == modified(&metadata)
        && cached.sha256 == model.sha256
    {
        return Ok(());
    }
    if hash_file(&mut File::open(file)?)? != model.sha256 {
        bail!("invalid GGUF checksum: {}", file.display());
    }
    atomic_json(
        Path::new(&cache),
        &Verification {
            size: metadata.len(),
            modified: modified(&metadata),
            sha256: model.sha256.clone(),
        },
    )
}
pub async fn prepare_model(alias: &str) -> Result<()> {
    valid_model(alias)?;
    if fs::metadata("/run/gpu/ready").is_err() {
        bail!("fastgpu environment is not initialized");
    }
    download_model(Path::new(MODEL_DIR), &preset(alias).unwrap()).await
}
pub async fn download_model(dir: &Path, model: &ModelPreset) -> Result<()> {
    fs::create_dir_all(dir)?;
    let _lock = lock_file(
        &dir.join("download.lock"),
        "a model download is already in progress",
    )?;
    let dest = dir.join(&model.file);
    match fs::metadata(&dest) {
        Ok(_) => {
            verify_model(&dest, model)?;
            println!("GGUF already present and verified");
            return Ok(());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let partial = dir.join(format!("{}.partial", model.file));
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o644)
        .open(&partial)?;
    let mut offset = file.metadata()?.len();
    if offset > model.size {
        bail!("partial download is larger than the model: move it before retrying");
    }
    let required = model.size - offset + (1 << 30);
    if fs2::available_space(dir)? < required {
        bail!(
            "insufficient volume space: at least {:.1} GB of free space required",
            required as f64 / 1e9
        );
    }
    if offset < model.size {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(3 * 60 * 60))
            .build()?;
        let mut request = client.get(&model.url).header("Accept-Encoding", "identity");
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let mut response = request.send().await?;
        let status = response.status().as_u16();
        if status != 200 && status != 206 {
            bail!("GGUF download: HTTP {status}");
        }
        if status == 206
            && !response
                .headers()
                .get("Content-Range")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .starts_with(&format!("bytes {offset}-"))
        {
            bail!("invalid download Range response");
        }
        if status == 200 {
            offset = 0;
            file.set_len(0)?;
        }
        file.seek(SeekFrom::Start(offset))?;
        let mut last = Instant::now();
        while let Some(chunk) = response.chunk().await? {
            let n = chunk.len().min((model.size - offset + 1) as usize);
            file.write_all(&chunk[..n])?;
            offset += n as u64;
            if last.elapsed() > Duration::from_secs(5) {
                eprintln!(
                    "GGUF: {:.1} / {:.1} GB",
                    offset as f64 / 1e9,
                    model.size as f64 / 1e9
                );
                last = Instant::now();
            }
            if offset > model.size {
                break;
            }
        }
    }
    file.sync_all()?;
    drop(file);
    verify_model(&partial, model)?;
    fs::rename(&partial, &dest)?;
    let _ = fs::rename(
        format!("{}.verified.json", partial.display()),
        format!("{}.verified.json", dest.display()),
    );
    println!("GGUF downloaded and SHA-256 verified");
    Ok(())
}
pub async fn supervisor(action: &str) -> Result<()> {
    process::command(
        "supervisorctl",
        &[
            "-c",
            "/etc/supervisor/supervisord.conf",
            action,
            "inference",
        ],
    )
    .await
}
pub fn worker_serve() -> Result<()> {
    let deployment: Deployment = serde_json::from_slice(&fs::read(DEPLOY_FILE)?)?;
    validate_deployment(&deployment)?;
    if deployment_fingerprint(&deployment)? != deployment.sha256 {
        bail!("model changed: run fastgpu deploy again before restarting the service");
    }
    let (name, args) = engine_command(&deployment);
    let error = std::process::Command::new(name)
        .args(args)
        .envs(engine_env())
        .exec();
    Err(error).context("could not execute inference engine")
}
pub async fn worker_check() -> Result<()> {
    let checks = vec![
        vec!["python", "--version"],
        vec!["uv", "--version"],
        vec!["gh", "--version"],
        vec!["codex", "--version"],
        vec!["llama-server", "--version"],
        vec!["ollama", "--version"],
        vec![
            "nvidia-smi",
            "--query-gpu=name,memory.total,driver_version",
            "--format=csv,noheader",
        ],
        vec![
            "python",
            "-c",
            "import torch, vllm; print('vLLM', vllm.__version__, 'PyTorch', torch.__version__, 'CUDA', torch.version.cuda); assert torch.cuda.is_available(), 'CUDA is not available'; x=torch.ones((32,32),device='cuda'); assert (x@x)[0,0].item()==32; print('CUDA computation succeeded')",
        ],
    ];
    for args in checks {
        println!("Checking: {}", args[0]);
        process::command(args[0], &args[1..]).await?;
    }
    Ok(())
}

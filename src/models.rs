use crate::{engines::persistent_path, ssh::quote};
use anyhow::{Result, bail};
use std::{io::Write, path::Path};
pub const MODEL_ALIAS: &str = "qwen3.8-27b";
pub const MODEL_DIR: &str = "/workspace/.gpu/models";
pub const DEPLOY_FILE: &str = "/workspace/.gpu/deployment.json";
#[derive(Clone)]
pub struct ModelPreset {
    pub file: String,
    pub url: String,
    pub sha256: String,
    pub size: u64,
}
pub fn preset_aliases() -> [&'static str; 2] {
    ["nanbeige4.3-33b", MODEL_ALIAS]
}
pub fn preset(alias: &str) -> Option<ModelPreset> {
    let (repo, revision, file, sha256, size) = match alias {
        MODEL_ALIAS => (
            "unsloth/Qwen3.8-27B-GGUF",
            "4ca720788d1e01f1bff70c033e0d0028fd02e502",
            "Qwen3.8-27B-UD-Q4_K_M.gguf",
            "322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482",
            16464440224,
        ),
        "nanbeige4.3-33b" => (
            "bartowski/Nanbeige_Nanbeige4.3-33B-GGUF",
            "9beb68421fdf7be095093a6888325fa2c83ddd38",
            "Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf",
            "2cef79855ca47ee6571e166db32a07089e34f3cf78e0397b262d124ddb814c37",
            20788825216,
        ),
        _ => return None,
    };
    Some(ModelPreset {
        file: file.into(),
        url: format!("https://huggingface.co/{repo}/resolve/{revision}/{file}"),
        sha256: sha256.into(),
        size,
    })
}
pub fn valid_model(alias: &str) -> Result<()> {
    if preset(alias).is_none() {
        bail!("supported models: {}", preset_aliases().join(", "));
    }
    Ok(())
}
pub fn list_models(out: &mut impl Write, volume: &Path, directory: &Path) -> Result<()> {
    let root = volume.canonicalize()?;
    let dir = persistent_path(&root, directory)?;
    let mut rows = Vec::new();
    for entry in walkdir::WalkDir::new(&dir)
        .sort_by_file_name()
        .follow_links(false)
    {
        let entry = entry?;
        if !entry.file_type().is_file()
            || !entry
                .file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".gguf")
        {
            continue;
        }
        let target = preset_aliases()
            .into_iter()
            .find(|alias| {
                entry.path() == root.join(".gpu/models").join(preset(alias).unwrap().file)
            })
            .map(str::to_string)
            .unwrap_or_else(|| quote(&entry.path().to_string_lossy()));
        rows.push((target, entry.metadata()?.len() as f64 / 1e9));
    }
    if rows.is_empty() {
        writeln!(out, "No GGUF models found in {}", dir.display())?;
        return Ok(());
    }
    let width = rows.iter().map(|r| r.0.len()).max().unwrap_or(0).max(13) + 2;
    writeln!(out, "{:<width$}SIZE (GB)", "DEPLOY TARGET")?;
    for (target, size) in rows {
        writeln!(out, "{target:<width$}{size:.2}")?;
    }
    writeln!(out, "\nDeploy with: fastgpu deploy <DEPLOY TARGET>")?;
    Ok(())
}

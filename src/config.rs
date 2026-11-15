use crate::api::Api;
use anyhow::{Context, Result, bail};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REMOTE_HOME: &str = "/home/gpu";
pub const WORKSPACE: &str = "/workspace";
pub const DEFAULT_IMAGE: &str = "felipeoperti/fastgpu-workspace:0.3.0";
fn zero(n: &i64) -> bool {
    *n == 0
}
fn zero_float(n: &f64) -> bool {
    *n == 0.0
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub image: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub registry: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub volume: String,
    pub ssh_key: String,
    pub min_vram_gb: i64,
    pub min_ram_gb: i64,
    pub min_cuda: String,
    pub container_disk_gb: i64,
    #[serde(skip_serializing_if = "zero_float")]
    pub max_price: f64,
    pub context_tokens: i64,
    pub local_port: i64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            image: DEFAULT_IMAGE.into(),
            registry: String::new(),
            volume: String::new(),
            ssh_key: String::new(),
            min_vram_gb: 24,
            min_ram_gb: 32,
            min_cuda: "13.0".into(),
            container_disk_gb: 40,
            max_price: 0.0,
            context_tokens: 131072,
            local_port: 8080,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pod_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub pending_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub volume_id: String,
    pub managed: bool,
    #[serde(skip_serializing_if = "zero")]
    pub tunnel_port: i64,
}
pub struct App {
    pub config: Config,
    pub state: State,
    pub dir: PathBuf,
    pub api: Option<Api>,
    lock: Option<FileLock>,
}

pub struct FileLock(File);
impl Drop for FileLock {
    fn drop(&mut self) {
        // Explicitly unlock before closing, including when a forked child has
        // temporarily inherited the descriptor before exec closes it.
        let _ = FileExt::unlock(&self.0);
    }
}
impl App {
    pub fn new() -> Result<Self> {
        let dir = match env::var_os("GPU_CONFIG_DIR").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir),
            None => env::var_os("XDG_CONFIG_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
                .unwrap_or(home()?.join(".config"))
                .join("gpu"),
        };
        Self::load(&std::path::absolute(dir)?)
    }
    pub fn load(dir: &Path) -> Result<Self> {
        let mut defaults = serde_json::to_value(Config {
            ssh_key: dir.join("id_ed25519").to_string_lossy().into(),
            ..Config::default()
        })?;
        if let Some(value) = read_optional::<serde_json::Value>(&dir.join("config.json"))? {
            let object = value
                .as_object()
                .context("config.json: expected an object")?;
            defaults.as_object_mut().unwrap().extend(object.clone());
        }
        Ok(Self {
            config: serde_json::from_value(defaults).context("config.json")?,
            state: read_optional(&dir.join("state.json"))?.unwrap_or_default(),
            dir: dir.into(),
            api: None,
            lock: None,
        })
    }
    pub fn client(&self) -> Result<Api> {
        self.api.clone().map_or_else(Api::new, Ok)
    }
    pub fn acquire(&mut self) -> Result<()> {
        if self.lock.is_none() {
            self.lock = Some(lock_file(
                &self.dir.join("lock"),
                "another fastgpu operation is modifying this profile",
            )?);
        }
        Ok(())
    }
    pub fn save_config(&self) -> Result<()> {
        atomic_json(&self.dir.join("config.json"), &self.config)
    }
    pub fn save_state(&self) -> Result<()> {
        atomic_json(&self.dir.join("state.json"), &self.state)
    }
}
pub fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("home directory unavailable")
}
pub fn private_dir(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    Ok(())
}
pub fn lock_file(path: &Path, message: &str) -> Result<FileLock> {
    private_dir(path.parent().context("missing parent directory")?)?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.try_lock_exclusive()
        .map_err(|_| anyhow::anyhow!("{message}"))?;
    Ok(FileLock(file))
}
pub fn read_optional<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(data) => Ok(Some(
            serde_json::from_slice(&data).with_context(|| path.display().to_string())?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut data = serde_json::to_vec_pretty(value)?;
    data.push(b'\n');
    atomic_file(path, &data)
}
pub fn atomic_file(path: &Path, data: &[u8]) -> Result<()> {
    let parent = path.parent().context("missing parent directory")?;
    private_dir(parent)?;
    let mut temp = tempfile::Builder::new()
        .prefix(".gpu-")
        .tempfile_in(parent)?;
    temp.write_all(data)?;
    temp.as_file().sync_all()?;
    temp.persist(path)?;
    Ok(())
}
pub fn random_name() -> String {
    format!("gpu-{:012x}", rand::random::<u64>() & 0xffffffffffff)
}
pub fn api_key() -> Result<String> {
    if let Ok(key) = env::var("RUNPOD_API_KEY")
        && !key.trim().is_empty()
    {
        return Ok(key.trim().into());
    }
    let data = fs::read_to_string(home()?.join(".runpod/config.toml"))
        .map_err(|_| anyhow::anyhow!("set RUNPOD_API_KEY or configure runpodctl"))?;
    #[derive(Deserialize)]
    struct Credentials {
        #[serde(default)]
        apikey: String,
    }
    let credentials: Credentials =
        toml::from_str(&data).map_err(|_| anyhow::anyhow!("invalid runpodctl configuration"))?;
    if credentials.apikey.is_empty() {
        bail!("missing RunPod key");
    }
    Ok(credentials.apikey)
}
pub fn print_json(value: &impl Serialize) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

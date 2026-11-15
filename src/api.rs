use crate::config::api_key;
use anyhow::{Context, Result, bail};
use reqwest::{Client, Method};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::HashSet, fmt, io::Write, time::Duration};

#[derive(Clone)]
pub struct Api {
    pub base: String,
    pub key: String,
    pub http: Client,
}
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub detail: String,
}
impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RunPod HTTP {}: {}", self.status, self.detail)
    }
}
impl std::error::Error for ApiError {}
pub fn is_status(error: &anyhow::Error, status: u16) -> bool {
    error
        .downcast_ref::<ApiError>()
        .is_some_and(|e| e.status == status)
}
pub fn escape(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}
impl Api {
    pub fn new() -> Result<Self> {
        Ok(Self {
            base: "https://api.runpod.io/v2".into(),
            key: api_key()?,
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    pub async fn request(&self, method: Method, path: &str, body: Option<&Value>) -> Result<Value> {
        for attempt in 0..3 {
            let mut request = self
                .http
                .request(method.clone(), format!("{}{path}", self.base))
                .timeout(Duration::from_secs(45))
                .bearer_auth(&self.key)
                .header("Content-Type", "application/json");
            if let Some(body) = body {
                request = request.json(body);
            }
            let mut response = request.send().await?;
            let status = response.status();
            let mut data = Vec::new();
            while let Some(chunk) = response.chunk().await? {
                if data.len() + chunk.len() > 8 << 20 {
                    bail!("RunPod response exceeds 8 MiB");
                }
                data.extend_from_slice(&chunk);
            }
            if status.is_success() {
                return if data.is_empty() {
                    Ok(Value::Null)
                } else {
                    Ok(serde_json::from_slice(&data)?)
                };
            }
            if method == Method::GET
                && (status.as_u16() == 429 || status.is_server_error())
                && attempt < 2
            {
                tokio::time::sleep(Duration::from_secs(attempt + 1)).await;
                continue;
            }
            let value: Value = serde_json::from_slice(&data).unwrap_or_default();
            let detail = value["detail"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or_else(|| value["title"].as_str())
                .unwrap_or(status.canonical_reason().unwrap_or("unknown error"));
            return Err(ApiError {
                status: status.as_u16(),
                detail: if self.key.is_empty() {
                    detail.into()
                } else {
                    detail.replace(&self.key, "[redacted]")
                },
            }
            .into());
        }
        bail!("RunPod: retries exhausted")
    }
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        Ok(serde_json::from_value(
            self.request(Method::GET, path, None).await?,
        )?)
    }
    pub async fn list<T: DeserializeOwned>(&self, path: &str, key: &str) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut cursor = String::new();
        let mut seen = HashSet::new();
        loop {
            let path = if cursor.is_empty() {
                path.to_string()
            } else {
                format!("{path}?cursor={}", escape(&cursor))
            };
            let response = self.request(Method::GET, &path, None).await?;
            items.extend(serde_json::from_value::<Vec<T>>(
                response.get(key).context("missing RunPod list")?.clone(),
            )?);
            if !response["pagination"]["hasNextPage"]
                .as_bool()
                .unwrap_or(false)
            {
                break;
            }
            cursor = response["pagination"]["nextCursor"]
                .as_str()
                .unwrap_or("")
                .into();
            if cursor.is_empty() || !seen.insert(cursor.clone()) {
                bail!("invalid RunPod pagination");
            }
        }
        Ok(items)
    }
    pub async fn pods(&self) -> Result<Vec<Pod>> {
        self.list("/pods", "pods").await
    }
    pub async fn volumes(&self) -> Result<Vec<Volume>> {
        self.list("/network-volumes", "networkVolumes").await
    }
    pub async fn pod(&self, id: &str) -> Result<Pod> {
        self.get(&format!("/pods/{}", escape(id))).await
    }
    pub async fn action(&self, id: &str, action: &str) -> Result<()> {
        self.request(
            Method::POST,
            &format!("/pods/{}/action", escape(id)),
            Some(&json!({"action": action})),
        )
        .await?;
        Ok(())
    }
    pub async fn logs(&self, id: &str, out: &mut impl Write) -> Result<()> {
        let mut response = self
            .http
            .get(format!("{}/pods/{}/logs?tail=100", self.base, escape(id)))
            .bearer_auth(&self.key)
            .header("Accept", "text/event-stream")
            .send()
            .await?;
        if response.status().as_u16() != 200 {
            return Err(ApiError {
                status: response.status().as_u16(),
                detail: response
                    .status()
                    .canonical_reason()
                    .unwrap_or("unknown error")
                    .into(),
            }
            .into());
        }
        let mut buffer = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            for byte in chunk {
                if byte == b'\n' {
                    log_line(&buffer, out)?;
                    buffer.clear();
                } else {
                    buffer.push(byte);
                    if buffer.len() > 1 << 20 {
                        bail!("log line exceeds 1 MiB");
                    }
                }
            }
        }
        if !buffer.is_empty() {
            log_line(&buffer, out)?;
        }
        Ok(())
    }
}
fn log_line(line: &[u8], out: &mut impl Write) -> Result<()> {
    if let Some(data) = std::str::from_utf8(line)?.strip_prefix("data:") {
        let value: Value = serde_json::from_str(data.trim())?;
        writeln!(
            out,
            "[{}] {}",
            value["source"].as_str().unwrap_or(""),
            value["line"].as_str().unwrap_or("")
        )?;
    }
    Ok(())
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Volume {
    pub id: String,
    pub name: String,
    pub size: i64,
    pub data_center: String,
    #[serde(rename = "type")]
    pub tier: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Mount {
    pub volume_id: String,
    pub path: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Endpoint {
    pub host: String,
    pub port: i64,
    pub username: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Mounts {
    pub network: Vec<Mount>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Ssh {
    pub direct: Option<Endpoint>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PodGpu {
    pub id: String,
    pub count: i64,
    pub memory: i64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Pod {
    pub id: String,
    pub name: String,
    pub status: String,
    pub image: String,
    pub cost: f64,
    pub data_center_id: String,
    pub created_at: String,
    pub mounts: Mounts,
    pub ssh: Ssh,
    pub gpu: PodGpu,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Price {
    pub secure: f64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DataCenter {
    pub id: String,
    pub availability: String,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Gpu {
    pub id: String,
    pub name: String,
    pub manufacturer: String,
    pub memory: i64,
    pub secure: bool,
    pub price: Price,
    pub data_centers: Vec<DataCenter>,
}
pub fn resolve_volume(volumes: &[Volume], selector: &str) -> Result<Volume> {
    if let Some(volume) = volumes.iter().find(|v| v.id == selector) {
        return Ok(volume.clone());
    }
    let matches: Vec<_> = volumes.iter().filter(|v| v.name == selector).collect();
    match matches.as_slice() {
        [volume] => Ok((*volume).clone()),
        [] => bail!("volume {selector:?} not found"),
        _ => bail!("ambiguous volume name: use the ID"),
    }
}
pub fn eligible(gpus: Vec<Gpu>, dc: &str, min_vram: i64, max_price: f64) -> Vec<Gpu> {
    let mut result: Vec<_> = gpus
        .into_iter()
        .filter(|g| {
            g.secure
                && g.manufacturer == "NVIDIA"
                && g.memory >= min_vram
                && g.price.secure > 0.0
                && (max_price <= 0.0 || g.price.secure <= max_price)
                && supported_architecture(&g.id)
                && g.data_centers
                    .iter()
                    .any(|d| d.id == dc && d.availability != "NONE" && !d.availability.is_empty())
        })
        .collect();
    result.sort_by(|a, b| {
        a.price
            .secure
            .total_cmp(&b.price.secure)
            .then(a.id.cmp(&b.id))
    });
    result
}
pub fn size_offers(gpus: Vec<Gpu>, size: &str) -> Result<Vec<Gpu>> {
    if !["small", "medium", "large", "xlarge"].contains(&size) {
        bail!("unknown GPU size {size:?}");
    }
    let mut result: Vec<_> = gpus
        .into_iter()
        .filter(|g| {
            let p = g.price.secure;
            match size {
                "small" => (0.0..1.5).contains(&p),
                "medium" => (1.5..3.5).contains(&p),
                "large" => (3.5..=5.0).contains(&p),
                _ => p > 5.0,
            }
        })
        .collect();
    if size == "medium" {
        result.sort_by(|a, b| {
            (a.price.secure - 3.0)
                .abs()
                .total_cmp(&(b.price.secure - 3.0).abs())
        });
    }
    Ok(result)
}
fn supported_architecture(id: &str) -> bool {
    let legacy = ["V100", "P100", "P40", "K80", "T4", "RTX 8000"]
        .iter()
        .any(|s| id.contains(s));
    let turing = ["RTX 4000", "RTX 5000", "RTX 6000"]
        .iter()
        .any(|s| id.contains(s))
        && !id.contains("Ada")
        && !id.contains("PRO");
    !(legacy || turing)
}

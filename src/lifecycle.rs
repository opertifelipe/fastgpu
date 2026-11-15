use crate::{
    api::{Api, Gpu, Pod, Volume, eligible, escape, is_status, resolve_volume, size_offers},
    config::{App, State, WORKSPACE, print_json, random_name},
};
use anyhow::{Context, Result, bail};
use reqwest::Method;
use serde_json::json;
use std::{
    fs,
    io::{self, IsTerminal, Write},
    time::Duration,
};

pub fn choose(labels: &[String]) -> Result<usize> {
    if !io::stdin().is_terminal() {
        bail!("ambiguous selection without a terminal: specify an ID or path");
    }
    for (i, label) in labels.iter().enumerate() {
        eprintln!("{}) {label}", i + 1);
    }
    eprint!("Selection: ");
    io::stderr().flush()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line)?;
    let index: usize = line.trim().parse().context("invalid selection")?;
    if index == 0 || index > labels.len() {
        bail!("invalid selection");
    }
    Ok(index - 1)
}
pub fn mounted(pod: &Pod, id: &str) -> bool {
    pod.mounts.network.iter().any(|m| m.volume_id == id)
}
pub fn capacity_rejected(error: &anyhow::Error) -> bool {
    let text = format!("{error:#}").to_lowercase();
    is_status(error, 400)
        && (text.contains("capacity") || text.contains("no longer any instances available"))
}
impl App {
    pub async fn select_volume(&self, api: &Api, selector: &str) -> Result<Volume> {
        let volumes = api.volumes().await?;
        let selector = if selector.is_empty() {
            &self.config.volume
        } else {
            selector
        };
        if !selector.is_empty() {
            return resolve_volume(&volumes, selector);
        }
        match volumes.as_slice() {
            [] => bail!("no Network Volume: use fastgpu volumes create"),
            [volume] => Ok(volume.clone()),
            _ => {
                let labels: Vec<_> = volumes
                    .iter()
                    .map(|v| format!("{} ({}) — {} GB, {}", v.name, v.id, v.size, v.data_center))
                    .collect();
                Ok(volumes[choose(&labels)?].clone())
            }
        }
    }
    async fn offers(&self, api: &Api, volume: &Volume, size: &str) -> Result<Vec<Gpu>> {
        let value = api.request(Method::GET, &format!("/catalog/gpus?include=AVAILABILITY&product=POD&count=1&cloud=SECURE&minCudaVersion={}", escape(&self.config.min_cuda)), None).await?;
        let gpus = serde_json::from_value(value["gpus"].clone())?;
        let offers = size_offers(
            eligible(
                gpus,
                &volume.data_center,
                self.config.min_vram_gb,
                self.config.max_price,
            ),
            size,
        )?;
        if offers.is_empty() {
            bail!(
                "no compatible {size} On-Demand NVIDIA GPU available in {} within the size's price range and configured price cap (VRAM >= {} GB, CUDA >= {})",
                volume.data_center,
                self.config.min_vram_gb,
                self.config.min_cuda
            );
        }
        Ok(offers)
    }
    async fn await_ready(&self, api: &Api, id: &str) -> Result<Pod> {
        tokio::time::timeout(Duration::from_secs(25 * 60), async {
            let mut last = String::new();
            loop {
                let pod = api.pod(id).await?;
                if pod.status != last {
                    eprintln!("Pod {id}: {}", pod.status);
                    last.clone_from(&pod.status);
                }
                if ["EXITED", "TERMINATED"].contains(&pod.status.as_str()) {
                    bail!("Pod {id} stopped during startup");
                }
                if pod.status == "RUNNING"
                    && pod.ssh.direct.is_some()
                    && self
                        .remote_output(&pod, &["test", "-f", "/run/gpu/ready"])
                        .await
                        .is_ok()
                {
                    return Ok(pod);
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        })
        .await
        .with_context(|| {
            format!(
                "Pod {id} not ready; check fastgpu status and fastgpu logs --pod, or fastgpu down"
            )
        })?
    }
    pub async fn up(
        &mut self,
        selector: &str,
        size: &str,
        dry: bool,
        skip_model: bool,
    ) -> Result<Pod> {
        let api = self.client()?;
        loop {
            if !self.state.pod_id.is_empty() {
                match api.pod(&self.state.pod_id).await {
                    Ok(pod) if pod.status != "TERMINATED" => {
                        if !selector.is_empty()
                            && !mounted(&pod, &self.select_volume(&api, selector).await?.id)
                        {
                            bail!(
                                "Pod {} already exists and uses another volume; run fastgpu down before creating a new machine",
                                pod.id
                            );
                        }
                        eprintln!(
                            "Pod {} already exists ({}); reusing this machine. Run fastgpu down before creating a new machine or changing its size.",
                            pod.id, pod.status
                        );
                        if dry {
                            print_json(&pod)?;
                            return Ok(pod);
                        }
                        if pod.status == "EXITED" {
                            api.action(&pod.id, "start").await?;
                        }
                        let pod = self.await_ready(&api, &pod.id).await?;
                        if !skip_model {
                            self.remote(
                                &pod,
                                &["fastgpu", "worker", "prepare"],
                                None,
                                false,
                                false,
                                false,
                            )
                            .await?;
                        }
                        return Ok(pod);
                    }
                    Err(e) if !is_status(&e, 404) => return Err(e),
                    _ => {
                        self.state = State::default();
                        self.save_state()?;
                    }
                }
            }
            if self.state.pending_name.is_empty() {
                break;
            }
            let found: Vec<_> = api
                .pods()
                .await?
                .into_iter()
                .filter(|p| p.name == self.state.pending_name && p.status != "TERMINATED")
                .collect();
            if found.len() != 1 {
                bail!(
                    "previous creation has an uncertain outcome ({}): found {} Pods. Check fastgpu pods; use fastgpu use ID or fastgpu recover --clear only after verification",
                    self.state.pending_name,
                    found.len()
                );
            }
            self.state.pod_id.clone_from(&found[0].id);
            self.state.managed = true;
            self.state.pending_name.clear();
            self.save_state()?;
        }
        for pod in api.pods().await? {
            if pod.name.starts_with("gpu-") && pod.status != "TERMINATED" {
                bail!(
                    "Pod {} already exists ({}); only one fastgpu machine is allowed. Run fastgpu down from its profile, or recover it with fastgpu use {} before running fastgpu down",
                    pod.id,
                    pod.status,
                    pod.id
                );
            }
        }
        let volume = self.select_volume(&api, selector).await?;
        let offers = self.offers(&api, &volume, size).await?;
        let config = &self.config;
        if config.container_disk_gb < 20
            || config.context_tokens < 512
            || config.min_vram_gb < 1
            || config.min_ram_gb < 1
            || config.image.is_empty()
        {
            bail!("invalid configuration: check fastgpu config");
        }
        eprintln!(
            "Volume {} ({}), {}; size {size}; GPU {}, {} GB VRAM; On-Demand list price ${:.3}/h + storage",
            volume.name,
            volume.id,
            volume.data_center,
            offers[0].name,
            offers[0].memory,
            offers[0].price.secure
        );
        if dry {
            print_json(
                &json!({"volume":volume,"size":size,"offers":offers,"image":config.image,"disk_gb":config.container_disk_gb,"min_ram_gb":config.min_ram_gb}),
            )?;
            return Ok(Pod::default());
        }
        self.ensure_key().await?;
        let public_key = fs::read_to_string(format!("{}.pub", config.ssh_key))?;
        self.state = State {
            pending_name: random_name(),
            volume_id: volume.id.clone(),
            managed: true,
            ..State::default()
        };
        self.save_state()?;
        let mut body = json!({"name": self.state.pending_name,"image":config.image,"cloud":"SECURE","dataCenterIds":[volume.data_center],"disk":config.container_disk_gb,"ports":["22/tcp"],"env":{"PUBLIC_KEY":public_key.trim(),"GPU_WORKSPACE_ID":self.state.pending_name},"mounts":{"network":[{"volumeId":volume.id,"path":WORKSPACE}]}});
        if !config.registry.is_empty() {
            body["registry"] = json!(config.registry);
        }
        let mut created = None;
        for (i, offer) in offers.iter().enumerate() {
            body["gpu"] = json!({"id":offer.id,"count":1,"minRamPerGpu":config.min_ram_gb,"minVcpuCountPerGpu":4,"minCudaVersion":config.min_cuda});
            match api.request(Method::POST, "/pods", Some(&body)).await {
                Ok(value) => {
                    created = Some(serde_json::from_value::<Pod>(value)?);
                    break;
                }
                Err(e) => {
                    // Only an explicit capacity rejection permits trying another GPU.
                    if capacity_rejected(&e) && i + 1 < offers.len() {
                        eprintln!("Capacity exhausted; trying {}", offers[i + 1].name);
                        continue;
                    }
                    if [400, 401, 403, 422].iter().any(|s| is_status(&e, *s)) {
                        self.state = State::default();
                        let _ = self.save_state();
                    }
                    return Err(e).with_context(|| {
                        format!(
                            "creation (operation {})",
                            body["name"].as_str().unwrap_or("")
                        )
                    });
                }
            }
        }
        let pod = created.context("creation returned no Pod")?;
        if pod.id.is_empty() {
            bail!("creation response has no ID; check fastgpu pods before retrying");
        }
        self.state.pod_id.clone_from(&pod.id);
        self.state.pending_name.clear();
        self.save_state()
            .with_context(|| format!("Pod {} created but saving local state failed", pod.id))?;
        eprintln!(
            "Created {}; actual compute cost ${:.3}/h. To terminate it: fastgpu down",
            pod.id, pod.cost
        );
        if config.max_price > 0.0 && pod.cost > config.max_price {
            api.request(Method::DELETE, &format!("/pods/{}", escape(&pod.id)), None)
                .await
                .context("cost exceeds the limit and termination failed")?;
            self.state = State::default();
            let _ = self.save_state();
            bail!("actual cost exceeds --max-price; Pod terminated, volume retained");
        }
        let pod = self.await_ready(&api, &pod.id).await?;
        if !skip_model {
            self.remote(
                &pod,
                &["fastgpu", "worker", "prepare"],
                None,
                false,
                false,
                false,
            )
            .await?;
        }
        Ok(pod)
    }
    pub async fn down(&mut self) -> Result<()> {
        let api = self.client()?;
        let pod = match self.selected(&api).await {
            Ok(pod) => pod,
            Err(e) if is_status(&e, 404) => {
                self.state = State::default();
                return self.save_state();
            }
            Err(e) => return Err(e),
        };
        if !self.state.managed
            || pod.mounts.network.len() != 1
            || !mounted(&pod, &self.state.volume_id)
        {
            bail!("termination refused: Pod identity/storage does not match the managed state");
        }
        self.close_tunnel(&pod).await;
        eprintln!(
            "Terminating Pod {}; Network Volume {} retained",
            pod.id, self.state.volume_id
        );
        api.request(Method::DELETE, &format!("/pods/{}", escape(&pod.id)), None)
            .await?;
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                match api.pod(&pod.id).await {
                    Ok(p) if p.status == "TERMINATED" => break,
                    Err(e) if is_status(&e, 404) => break,
                    Err(e) => return Err(e),
                    _ => {}
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Ok(())
        })
        .await
        .context("timed out waiting for Pod termination")??;
        let volume: Volume = api
            .get(&format!(
                "/network-volumes/{}",
                escape(&self.state.volume_id)
            ))
            .await
            .context("Pod terminated; volume verification failed")?;
        self.state = State::default();
        self.save_state()?;
        println!(
            "Pod terminated. Volume {} ({}), {} GB, is still present.",
            volume.name, volume.id, volume.size
        );
        Ok(())
    }
}

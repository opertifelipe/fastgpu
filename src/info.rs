use crate::{
    api::{Pod, Volume, escape},
    config::{App, REMOTE_HOME, WORKSPACE},
    ssh::shell_words,
};
use anyhow::{Result, bail};
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub fn public_pod_details(raw: &Value) -> Value {
    let mut out = serde_json::Map::new();
    // Environment, startup arguments and registry containers may hold credentials.
    for key in [
        "id",
        "name",
        "status",
        "image",
        "cloud",
        "dataCenterId",
        "cudaVersion",
        "gpu",
        "cpu",
        "disk",
        "mounts",
        "ports",
        "ssh",
        "runtime",
        "cost",
        "createdAt",
        "startedAt",
        "actions",
        "locked",
        "globalNetworking",
        "template",
    ] {
        if let Some(value) = raw.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    Value::Object(out)
}
impl App {
    pub fn connections(
        &self,
        pod: &Pod,
        local: i64,
        remote: i64,
    ) -> Result<BTreeMap<String, String>> {
        if !(1..=65535).contains(&local) || !(1..=65535).contains(&remote) {
            bail!("allowed ports: 1..65535");
        }
        let mut commands = BTreeMap::new();
        let Some(endpoint) = pod.ssh.direct.as_ref().filter(|_| pod.status == "RUNNING") else {
            commands.insert(
                "status".into(),
                "direct SSH unavailable: the Pod must be running with port 22/tcp published".into(),
            );
            return Ok(commands);
        };
        self.ssh_config(pod)?;
        let config = self.dir.join("ssh_config").to_string_lossy().into_owned();
        let known = self.dir.join("known_hosts").to_string_lossy().into_owned();
        let host = format!("gpu-{}", pod.id);
        let manual = shell_words(&[
            "ssh",
            "-tt",
            "-i",
            &self.config.ssh_key,
            "-p",
            &endpoint.port.to_string(),
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=accept-new",
            "-o",
            &format!("HostKeyAlias={host}"),
            "-o",
            &format!("UserKnownHostsFile={known}"),
            &format!("gpu@{}", endpoint.host),
        ]);
        let tcp = if endpoint.host.contains(':') {
            format!("[{}]:{}", endpoint.host, endpoint.port)
        } else {
            format!("{}:{}", endpoint.host, endpoint.port)
        };
        for (key, value) in [
            ("tcp", tcp),
            ("ssh", manual),
            (
                "ssh_config",
                shell_words(&["ssh", "-F", &config, "-tt", &host]),
            ),
            (
                "tunnel_tcp",
                shell_words(&[
                    "ssh",
                    "-F",
                    &config,
                    "-N",
                    "-o",
                    "ExitOnForwardFailure=yes",
                    "-L",
                    &format!("127.0.0.1:{local}:127.0.0.1:{remote}"),
                    &host,
                ]),
            ),
            ("local_endpoint", format!("127.0.0.1:{local}")),
        ] {
            commands.insert(key.into(), value);
        }
        if remote == 8080 {
            commands.insert("local_api".into(), format!("http://127.0.0.1:{local}/v1"));
        }
        Ok(commands)
    }
    pub async fn info(&self, live: bool) -> Result<Value> {
        if self.state.pod_id.is_empty() {
            bail!("no Pod selected: use fastgpu up");
        }
        let api = self.client()?;
        let raw: Value = api
            .get(&format!("/pods/{}", escape(&self.state.pod_id)))
            .await?;
        let pod: Pod = serde_json::from_value(raw.clone())?;
        let connections = self.connections(&pod, self.config.local_port, 8080)?;
        let mut volumes = Vec::new();
        for mount in &pod.mounts.network {
            volumes.push(
                api.get::<Volume>(&format!("/network-volumes/{}", escape(&mount.volume_id)))
                    .await?,
            );
        }
        let mut report = json!({"pod":public_pod_details(&raw),"volumes":volumes,"connections":connections,"persistence":{"mount":WORKSPACE,"home":format!("{REMOTE_HOME} -> /workspace/.gpu/home"),"repository":"/home/gpu/progetti","models":crate::models::MODEL_DIR,"huggingface_cache":"/home/gpu/.cache/huggingface","ollama":"/workspace/.gpu/ollama/models"},"note":"API gpu.memory is system RAM, not VRAM; compute rate is USD/hour, storage is billed separately"});
        if live {
            let script = "import json,os,platform,subprocess\nfrom pathlib import Path\ngpu=subprocess.run(['nvidia-smi','--query-gpu=name,memory.total,memory.used,utilization.gpu,driver_version','--format=csv'],capture_output=True,text=True)\nprint(json.dumps({'system':platform.platform(),'visible_cpus':os.cpu_count(),'resolved_home':os.path.realpath('/home/gpu'),'gpu_csv':gpu.stdout.strip(),'gpu_error':gpu.stderr.strip(),'kernel_memory':Path('/proc/meminfo').read_text().splitlines()[:3]}))";
            report["remote_system"] =
                serde_json::from_str(&self.remote_output(&pod, &["python", "-c", script]).await?)?;
        }
        Ok(report)
    }
}

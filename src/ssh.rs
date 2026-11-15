use crate::{
    api::{Api, Pod},
    config::{App, REMOTE_HOME, VERSION, WORKSPACE, atomic_file, home, private_dir},
    process::{self, Terminal},
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    net::TcpListener,
    path::{Component, Path, PathBuf},
    process::Stdio,
};
use tokio::process::Command;

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
pub fn shell_words<T: AsRef<str>>(args: &[T]) -> String {
    args.iter()
        .map(|s| quote(s.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}
pub fn clean_path(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                result.pop();
            }
            _ => result.push(part.as_os_str()),
        }
    }
    result
}
pub fn remote_path(s: &str) -> Result<String> {
    if s.contains(['\0', '\r', '\n']) {
        bail!("invalid path");
    }
    let (base, relative) = if let Some(s) = s.strip_prefix("vol:") {
        (WORKSPACE, s)
    } else {
        (REMOTE_HOME, s.strip_prefix("home:").unwrap_or(s))
    };
    if relative.starts_with('/') {
        bail!("use a relative path with home: or vol:");
    }
    let path = clean_path(&Path::new(base).join(relative));
    if !path.starts_with(base) {
        bail!("path escapes the selected root");
    }
    Ok(path.to_string_lossy().into())
}
pub fn transfer_path(s: &str) -> Result<String> {
    if Path::new(s).is_absolute() && !s.contains(['\0', '\r', '\n']) {
        return Ok(clean_path(Path::new(s)).to_string_lossy().into());
    }
    remote_path(s)
}
impl App {
    pub async fn check_worker(&self, pod: &Pod) -> Result<()> {
        let version = self.remote_output(pod, &["fastgpu", "--version"]).await?;
        if version.trim() != format!("fastgpu version {VERSION}") {
            bail!(
                "remote CLI version mismatch ({}): the image must contain fastgpu {VERSION}. Updating the local CLI does not update the Pod. Run fastgpu down (retains the Network Volume), fastgpu init --image YOUR_ACCOUNT/fastgpu-workspace:{VERSION}, then fastgpu up --small --skip-model",
                version.trim()
            );
        }
        Ok(())
    }
    pub fn ssh_config(&self, pod: &Pod) -> Result<()> {
        let endpoint = pod
            .ssh
            .direct
            .as_ref()
            .with_context(|| format!("Pod {}: direct SSH is not available yet", pod.id))?;
        let safe = |s: &str| {
            !s.is_empty()
                && s.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._:-".contains(&c))
        };
        if !safe(&endpoint.host) || !safe(&pod.id) || !(1..=65535).contains(&endpoint.port) {
            bail!("invalid SSH endpoint");
        }
        if format!("{}{}", self.config.ssh_key, self.dir.display()).contains(['\r', '\n', '\0']) {
            bail!("invalid SSH configuration path");
        }
        let key = serde_json::to_string(&self.config.ssh_key)?;
        let known = serde_json::to_string(&self.dir.join("known_hosts"))?;
        let content = format!(
            "Host gpu-{}\n  HostName {}\n  Port {}\n  User gpu\n  IdentityFile {key}\n  IdentitiesOnly yes\n  UserKnownHostsFile {known}\n  HostKeyAlias gpu-{}\n  StrictHostKeyChecking accept-new\n  ConnectTimeout 10\n  ServerAliveInterval 20\n  ServerAliveCountMax 3\n  ForwardAgent no\n",
            pod.id, endpoint.host, endpoint.port, pod.id
        );
        atomic_file(&self.dir.join("ssh_config"), content.as_bytes())
    }
    pub async fn remote(
        &self,
        pod: &Pod,
        args: &[&str],
        input: Option<&[u8]>,
        capture: bool,
        tty: bool,
        inherit_stdin: bool,
    ) -> Result<String> {
        self.ssh_config(pod)?;
        let _terminal = Terminal::save(tty)?;
        let mut cmd = Command::new("ssh");
        cmd.arg("-F")
            .arg(self.dir.join("ssh_config"))
            .args(["-o", "BatchMode=yes"]);
        if tty {
            cmd.arg("-tt");
        }
        cmd.arg(format!("gpu-{}", pod.id));
        if !args.is_empty() {
            cmd.arg(format!(
                "exec bash -lc {}",
                quote(&format!("exec {}", shell_words(args)))
            ));
        }
        process::run(&mut cmd, input, capture, inherit_stdin).await
    }
    pub async fn remote_output(&self, pod: &Pod, args: &[&str]) -> Result<String> {
        self.remote(pod, args, None, true, false, false).await
    }
    pub async fn selected(&self, api: &Api) -> Result<Pod> {
        if self.state.pod_id.is_empty() {
            bail!("no Pod selected: use fastgpu up");
        }
        api.pod(&self.state.pod_id).await
    }
    pub async fn ensure_key(&self) -> Result<()> {
        let key = Path::new(&self.config.ssh_key);
        match fs::metadata(key) {
            Ok(_) => {
                fs::metadata(format!("{}.pub", self.config.ssh_key))?;
                return Ok(());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        if let Some(parent) = key.parent().filter(|p| !p.as_os_str().is_empty()) {
            private_dir(parent)?;
        }
        process::command(
            "ssh-keygen",
            &[
                "-q",
                "-t",
                "ed25519",
                "-N",
                "",
                "-C",
                "fastgpu-workspace",
                "-f",
                &self.config.ssh_key,
            ],
        )
        .await
    }
    fn tunnel_command(&self, pod: &Pod) -> Command {
        let mut cmd = Command::new("ssh");
        cmd.arg("-F")
            .arg(self.dir.join("ssh_config"))
            .arg("-S")
            .arg(self.dir.join(format!("tunnel-{}", pod.id)));
        cmd
    }
    pub async fn tunnel(&mut self, pod: &Pod, port: i64) -> Result<()> {
        if !(1..=65535).contains(&port) {
            bail!("invalid local port");
        }
        self.ssh_config(pod)?;
        let host = format!("gpu-{}", pod.id);
        if process::run(
            self.tunnel_command(pod)
                .args(["-O", "check", &host])
                .stderr(Stdio::null()),
            None,
            true,
            false,
        )
        .await
        .is_ok()
        {
            if self.state.tunnel_port == port {
                return Ok(());
            }
            bail!(
                "tunnel already exists on port {}: close it with fastgpu tunnel --close",
                self.state.tunnel_port
            );
        }
        drop(
            TcpListener::bind(("127.0.0.1", port as u16))
                .with_context(|| format!("port {port} is in use: choose --local-port"))?,
        );
        process::run(
            self.tunnel_command(pod).args([
                "-M",
                "-fN",
                "-o",
                "BatchMode=yes",
                "-o",
                "ExitOnForwardFailure=yes",
                "-L",
                &format!("127.0.0.1:{port}:127.0.0.1:8080"),
                &host,
            ]),
            None,
            false,
            false,
        )
        .await?;
        self.state.tunnel_port = port;
        self.save_state()
    }
    pub async fn close_tunnel(&mut self, pod: &Pod) {
        if self.ssh_config(pod).is_err() {
            return;
        }
        let _ = process::run(
            self.tunnel_command(pod)
                .args(["-O", "exit", &format!("gpu-{}", pod.id)])
                .stderr(Stdio::null()),
            None,
            true,
            false,
        )
        .await;
        self.state.tunnel_port = 0;
    }
    pub async fn transfer(&self, pod: &Pod, local: &str, remote: &str, push: bool) -> Result<()> {
        let mut remote_path = transfer_path(remote)?;
        self.ssh_config(pod)?;
        let mut local_path = clean_path(&std::path::absolute(local)?)
            .to_string_lossy()
            .into_owned();
        if local.ends_with('/') {
            local_path.push('/');
        }
        if remote.ends_with('/') {
            remote_path.push('/');
        }
        let transport = shell_words(&[
            "ssh",
            "-F",
            &self.dir.join("ssh_config").to_string_lossy(),
            "-o",
            "BatchMode=yes",
        ]);
        let mut cmd = Command::new("rsync");
        cmd.args([
            "-a",
            "--no-owner",
            "--no-group",
            "--protect-args",
            "--partial",
            "--partial-dir=.gpu-rsync-partial",
            "--info=progress2",
            "-e",
            &transport,
        ]);
        for pattern in [".venv/", "__pycache__/", ".env", ".env.*", "*.pem", "*.key"] {
            cmd.args(["--exclude", pattern]);
        }
        if push {
            self.remote(
                pod,
                &["mkdir", "-p", "--", &remote_path],
                None,
                false,
                false,
                false,
            )
            .await?;
            cmd.args(["--", &local_path, &format!("gpu-{}:{remote_path}/", pod.id)]);
        } else {
            fs::create_dir_all(&local_path)?;
            cmd.args([
                "--",
                &format!("gpu-{}:{remote_path}", pod.id),
                &format!("{local_path}/"),
            ]);
        }
        process::run(&mut cmd, None, false, true).await?;
        Ok(())
    }
    pub async fn open_code(&self, pod: &Pod, folder: &str) -> Result<()> {
        self.ssh_config(pod)?;
        let remote = remote_path(folder)?;
        self.remote_output(pod, &["test", "-d", &remote])
            .await
            .with_context(|| format!("remote directory not found: {remote}"))?;
        let dir = home()?.join(".ssh");
        private_dir(&dir)?;
        let file = dir.join("config");
        let data = match fs::read_to_string(&file) {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        let include = format!(
            "Include {}",
            serde_json::to_string(&self.dir.join("ssh_config"))?
        );
        if !data.contains(&include) {
            use std::{io::Write, os::unix::fs::OpenOptionsExt};
            let mut out = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .mode(0o600)
                .open(&file)?;
            write!(out, "{include}\n{data}")?;
        }
        process::command(
            "code",
            &[
                "--new-window",
                "--remote",
                &format!("ssh-remote+gpu-{}", pod.id),
                &remote,
            ],
        )
        .await
    }
}

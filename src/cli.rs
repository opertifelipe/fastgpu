use crate::{
    api::{Volume, escape, resolve_volume},
    config::{App, State, VERSION, WORKSPACE, print_json},
    engines::{
        default_model_name, model_path, ready_model, validate_deployment, worker_deploy,
        worker_ready,
    },
    lifecycle::mounted,
    models::{MODEL_ALIAS, MODEL_DIR, list_models, preset, valid_model},
    process::find_program,
    ssh::{quote, remote_path, shell_words},
    worker::{Deployment, prepare_model, supervisor, worker_check, worker_serve},
};
use anyhow::{Context, Result, bail};
use clap::{Args, CommandFactory, Parser, Subcommand};
use reqwest::Method;
use serde_json::json;
use std::{io::Read, path::Path, time::Duration};

#[derive(Parser)]
#[command(name = "fastgpu", about = "Your remote GPU environment on RunPod", version = concat!("version ", env!("CARGO_PKG_VERSION")))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}
#[derive(Subcommand)]
pub enum Commands {
    /// Configure the image, SSH key, and volume
    Init {
        #[arg(long)]
        image: Option<String>,
        #[arg(long)]
        ssh_key: Option<String>,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long, default_value = "")]
        volume: String,
    },
    /// Show configuration without credentials
    Config,
    /// Create or resume one Pod (default size: small)
    Up(Up),
    /// List Pods without modifying them
    Pods,
    /// Show Pod details, status, and compute rate
    Status,
    /// Select a Pod with the fastgpu image and a Network Volume
    Use { id: String },
    /// Terminate the managed Pod and retain the Network Volume
    Down,
    /// Stop the Pod (volume billing continues)
    Stop,
    /// Clear an uncertain creation after manual verification
    Recover {
        #[arg(
            long,
            help = "Confirm that you also checked the console for the absent Pod"
        )]
        clear: bool,
    },
    /// Open an interactive shell or run a remote command
    Shell {
        #[arg(long, default_value = "home:")]
        cwd: String,
        #[arg(long)]
        tty: bool,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Create a persistent directory in the home directory or volume
    Mkdir { directory: String },
    /// Open a remote directory in local VS Code
    Code { directory: Option<String> },
    /// Copy with rsync without deleting destination files
    Push { source: String, destination: String },
    /// Copy with rsync without deleting destination files
    Pull { source: String, destination: String },
    /// Manage persistent GGUF models
    Model {
        #[command(subcommand)]
        command: Option<ModelCommand>,
    },
    /// Serve a GGUF or checkpoint with llama.cpp, Ollama, or vLLM
    Deploy(Deploy),
    /// Stop inference; the Pod remains running
    Undeploy,
    /// Open or close the model tunnel
    Tunnel {
        #[arg(long)]
        local_port: Option<i64>,
        #[arg(long)]
        close: bool,
    },
    /// Show model or container logs
    Logs {
        #[arg(long)]
        follow: bool,
        #[arg(long)]
        pod: bool,
    },
    /// Check dependencies and remote installations
    Doctor {
        #[arg(long)]
        remote: bool,
    },
    /// Show the compute rate and duration (not a bill)
    Cost,
    /// View and manage Network Volumes
    #[command(alias = "volume")]
    Volumes {
        #[command(subcommand)]
        command: Option<VolumeCommand>,
    },
    /// Show hardware, storage, status, networking, and connection commands
    Info {
        #[arg(long)]
        live: bool,
    },
    /// Print the TCP endpoint, SSH command, and tunnel command
    Connect {
        #[arg(long)]
        json: bool,
        #[arg(long)]
        local_port: Option<i64>,
        #[arg(long, default_value_t = 8080)]
        remote_port: i64,
    },
    #[command(hide = true)]
    Worker {
        #[command(subcommand)]
        command: WorkerCommand,
    },
}
#[derive(Args, Default)]
pub struct Up {
    #[arg(
        long,
        group = "size",
        help = "Cheapest compatible GPU below $1.50/hour (default)"
    )]
    small: bool,
    #[arg(
        long,
        group = "size",
        help = "GPU closest to $3/hour, from $1.50 to below $3.50/hour"
    )]
    medium: bool,
    #[arg(
        long,
        group = "size",
        help = "Cheapest compatible GPU from $3.50 to $5/hour"
    )]
    large: bool,
    #[arg(long, group = "size", help = "Cheapest compatible GPU above $5/hour")]
    xlarge: bool,
    #[arg(long, default_value = "", help = "Volume ID or name (override)")]
    volume: String,
    #[arg(long, help = "Show the plan without creating resources")]
    dry_run: bool,
    #[arg(long, help = "Skip the GGUF download")]
    skip_model: bool,
    #[arg(long)]
    max_price: Option<f64>,
    #[arg(long)]
    min_vram: Option<i64>,
    #[arg(long)]
    image: Option<String>,
    #[arg(long)]
    disk: Option<i64>,
}
impl Up {
    fn size(&self) -> &'static str {
        if self.medium {
            "medium"
        } else if self.large {
            "large"
        } else if self.xlarge {
            "xlarge"
        } else {
            "small"
        }
    }
}
#[derive(Args)]
pub struct Deploy {
    #[arg(value_name = "MODEL_OR_PATH", default_value = MODEL_ALIAS)]
    target: String,
    #[arg(
        long,
        default_value = "llama.cpp",
        help = "Engine: llama.cpp, ollama, vllm"
    )]
    engine: String,
    #[arg(long, default_value = "", help = "Model name in the local API")]
    name: String,
    #[arg(long, help = "Treat the model as a Hugging Face ID (vLLM)")]
    hf: bool,
    #[arg(long, default_value = "")]
    tokenizer: String,
    #[arg(long, default_value = "")]
    modelfile: String,
    #[arg(
        long = "arg",
        help = "Additional engine argument; repeatable, e.g. --arg=--enforce-eager"
    )]
    args: Vec<String>,
    #[arg(long)]
    min_vram: Option<i64>,
    #[arg(long)]
    context: Option<i64>,
    #[arg(long)]
    local_port: Option<i64>,
    #[arg(long)]
    max_price: Option<f64>,
}
#[derive(Subcommand)]
pub enum ModelCommand {
    /// Download and verify a preset: nanbeige4.3-33b, qwen3.8-27b
    Pull {
        #[arg(default_value = MODEL_ALIAS)]
        model: String,
    },
    /// List installed GGUFs with their deploy alias or path
    List { directory: Option<String> },
}
#[derive(Subcommand)]
pub enum WorkerCommand {
    Prepare {
        #[arg(default_value = MODEL_ALIAS)]
        model: String,
    },
    ListModels {
        #[arg(default_value = MODEL_DIR)]
        directory: String,
    },
    Serve,
    Check,
    Ready,
    Stop,
    Deploy,
}
#[derive(Subcommand)]
pub enum VolumeCommand {
    /// List Network Volumes
    List,
    /// Show volume details
    Show {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
    },
    /// Select the profile volume
    Use {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
    },
    /// Rename a volume
    Rename {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
        new_name: String,
    },
    /// Increase volume size
    Resize {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
        gb: i64,
    },
    /// Delete an unprotected volume with ID confirmation
    Delete {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
        #[arg(long, default_value = "")]
        confirm: String,
    },
    /// Create a Network Volume (paid storage)
    Create {
        name: String,
        #[arg(long, default_value_t = 0)]
        size: i64,
        #[arg(long, default_value = "")]
        datacenter: String,
        #[arg(long = "type", default_value = "STANDARD")]
        tier: String,
    },
}
pub async fn execute() -> Result<()> {
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
            ) =>
        {
            error.print()?;
            return Ok(());
        }
        Err(error) => return Err(error.into()),
    };
    App::new()?
        .dispatch(cli.command.unwrap_or(Commands::Up(Up::default())))
        .await
}
impl App {
    pub async fn dispatch(&mut self, command: Commands) -> Result<()> {
        if matches!(
            &command,
            Commands::Init { .. }
                | Commands::Up(_)
                | Commands::Use { .. }
                | Commands::Down
                | Commands::Stop
                | Commands::Recover { .. }
                | Commands::Deploy(_)
                | Commands::Undeploy
                | Commands::Tunnel { .. }
        ) {
            self.acquire()?;
        }
        match command {
            Commands::Init {
                image,
                ssh_key,
                registry,
                volume,
            } => {
                if let Some(v) = image {
                    self.config.image = v;
                }
                if let Some(v) = ssh_key {
                    self.config.ssh_key = v;
                }
                if let Some(v) = registry {
                    self.config.registry = v;
                }
                self.config.volume = self.select_volume(&self.client()?, &volume).await?.id;
                self.ensure_key().await?;
                self.save_config()?;
                println!(
                    "Configuration saved: {}",
                    self.dir.join("config.json").display()
                );
                print_json(&self.config)
            }
            Commands::Config => print_json(&self.config),
            Commands::Up(options) => {
                let size = options.size();
                if let Some(v) = options.max_price {
                    self.config.max_price = v;
                }
                if let Some(v) = options.min_vram {
                    self.config.min_vram_gb = v;
                }
                if let Some(v) = options.image {
                    self.config.image = v;
                }
                if let Some(v) = options.disk {
                    self.config.container_disk_gb = v;
                }
                self.up(&options.volume, size, options.dry_run, options.skip_model)
                    .await?;
                Ok(())
            }
            Commands::Pods => print_json(&self.client()?.pods().await?),
            Commands::Status => {
                if self.state.pod_id.is_empty() {
                    return print_json(&self.state);
                }
                print_json(&json!({"pod":self.selected(&self.client()?).await?,"local":self.state}))
            }
            Commands::Use { id } => {
                let pod = self.client()?.pod(&id).await?;
                if !pod.name.starts_with("gpu-") || pod.mounts.network.len() != 1 {
                    bail!("fastgpu use only accepts fastgpu Pods with a Network Volume");
                }
                self.remote_output(&pod, &["fastgpu", "--version"])
                    .await
                    .context("could not verify the fastgpu image")?;
                self.state = State {
                    pod_id: pod.id,
                    volume_id: pod.mounts.network[0].volume_id.clone(),
                    managed: true,
                    ..State::default()
                };
                self.save_state()
            }
            Commands::Down => self.down().await,
            Commands::Stop => {
                let api = self.client()?;
                let pod = self.selected(&api).await?;
                if !self.state.managed {
                    bail!("Pod is not managed");
                }
                self.close_tunnel(&pod).await;
                api.action(&pod.id, "stop").await?;
                self.save_state()
            }
            Commands::Recover { clear } => {
                if !clear {
                    return print_json(&self.state);
                }
                if !self.state.pod_id.is_empty() {
                    bail!("a Pod is selected: use down");
                }
                for pod in self.client()?.pods().await? {
                    if pod.name == self.state.pending_name {
                        bail!("Pod still exists: fastgpu use {}", pod.id);
                    }
                }
                self.state = State::default();
                self.save_state()
            }
            Commands::Shell {
                cwd,
                mut tty,
                mut command,
            } => {
                let pod = self.selected(&self.client()?).await?;
                let remote = remote_path(&cwd)?;
                if command.is_empty() {
                    command = vec!["bash".into(), "-l".into()];
                    tty = true;
                }
                self.remote(
                    &pod,
                    &[
                        "bash",
                        "-lc",
                        &format!("cd -- {} && exec {}", quote(&remote), shell_words(&command)),
                    ],
                    None,
                    false,
                    tty,
                    true,
                )
                .await?;
                Ok(())
            }
            Commands::Mkdir { directory } => {
                let pod = self.selected(&self.client()?).await?;
                self.remote(
                    &pod,
                    &["mkdir", "-p", "--", &remote_path(&directory)?],
                    None,
                    false,
                    false,
                    false,
                )
                .await?;
                Ok(())
            }
            Commands::Code { directory } => {
                self.open_code(
                    &self.selected(&self.client()?).await?,
                    directory.as_deref().unwrap_or(""),
                )
                .await
            }
            Commands::Push {
                source,
                destination,
            } => {
                self.transfer(
                    &self.selected(&self.client()?).await?,
                    &source,
                    &destination,
                    true,
                )
                .await
            }
            Commands::Pull {
                source,
                destination,
            } => {
                self.transfer(
                    &self.selected(&self.client()?).await?,
                    &destination,
                    &source,
                    false,
                )
                .await
            }
            Commands::Model { command } => {
                let Some(command) = command else {
                    let mut root = Cli::command();
                    root.find_subcommand_mut("model").unwrap().print_help()?;
                    println!();
                    return Ok(());
                };
                let args = match command {
                    ModelCommand::Pull { model } => {
                        valid_model(&model)?;
                        vec!["prepare".into(), model]
                    }
                    ModelCommand::List { directory } => vec![
                        "list-models".into(),
                        match directory {
                            Some(d) => model_path(&d)?,
                            None => MODEL_DIR.into(),
                        },
                    ],
                };
                let pod = self.selected(&self.client()?).await?;
                self.remote(
                    &pod,
                    &["fastgpu", "worker", &args[0], &args[1]],
                    None,
                    false,
                    false,
                    false,
                )
                .await?;
                Ok(())
            }
            Commands::Deploy(options) => self.deploy(options).await,
            Commands::Undeploy => {
                let pod = self.selected(&self.client()?).await?;
                self.close_tunnel(&pod).await;
                self.save_state()?;
                self.remote(
                    &pod,
                    &["fastgpu", "worker", "stop"],
                    None,
                    false,
                    false,
                    false,
                )
                .await?;
                Ok(())
            }
            Commands::Tunnel { local_port, close } => {
                let pod = self.selected(&self.client()?).await?;
                if close {
                    self.close_tunnel(&pod).await;
                    self.save_state()
                } else {
                    self.tunnel(&pod, local_port.unwrap_or(self.config.local_port))
                        .await
                }
            }
            Commands::Logs { follow, pod } => {
                let api = self.client()?;
                let selected = self.selected(&api).await?;
                if pod {
                    return api.logs(&selected.id, &mut std::io::stdout()).await;
                }
                let mut args = vec!["tail", "-n", "100"];
                if follow {
                    args.push("-F");
                }
                args.push("/workspace/.gpu/logs/inference.log");
                self.remote(&selected, &args, None, false, false, false)
                    .await?;
                Ok(())
            }
            Commands::Doctor { remote } => {
                for name in ["ssh", "rsync"] {
                    println!(
                        "{} {}",
                        name,
                        find_program(name)
                            .with_context(|| format!("{name} not found"))?
                            .display()
                    );
                }
                if let Some(path) = find_program("code") {
                    println!("code {}", path.display());
                } else {
                    println!("VS Code not found (optional; required by fastgpu code)");
                }
                let api = self.client()?;
                println!(
                    "RunPod API OK; {} Network Volumes",
                    api.volumes().await?.len()
                );
                if remote {
                    self.remote(
                        &self.selected(&api).await?,
                        &["fastgpu", "worker", "check"],
                        None,
                        false,
                        false,
                        false,
                    )
                    .await?;
                }
                Ok(())
            }
            Commands::Cost => {
                let pod = self.selected(&self.client()?).await?;
                print_json(
                    &json!({"pod":pod.id,"status":pod.status,"compute_usd_per_hour":pod.cost,"created_at":pod.created_at,"note":"Storage is billed separately; the compute rate does not imply billing while EXITED"}),
                )
            }
            Commands::Volumes { command } => {
                self.volumes(command.unwrap_or(VolumeCommand::List)).await
            }
            Commands::Info { live } => print_json(&self.info(live).await?),
            Commands::Connect {
                json,
                local_port,
                remote_port,
            } => {
                let commands = self.connections(
                    &self.selected(&self.client()?).await?,
                    local_port.unwrap_or(self.config.local_port),
                    remote_port,
                )?;
                if json {
                    return print_json(&commands);
                }
                for key in [
                    "status",
                    "tcp",
                    "ssh",
                    "ssh_config",
                    "tunnel_tcp",
                    "local_endpoint",
                    "local_api",
                ] {
                    if let Some(value) = commands.get(key) {
                        println!("{key}: {value}");
                    }
                }
                Ok(())
            }
            Commands::Worker { command } => match command {
                WorkerCommand::Prepare { model } => prepare_model(&model).await,
                WorkerCommand::ListModels { directory } => list_models(
                    &mut std::io::stdout(),
                    Path::new(WORKSPACE),
                    Path::new(&directory),
                ),
                WorkerCommand::Serve => worker_serve(),
                WorkerCommand::Check => worker_check().await,
                WorkerCommand::Ready => worker_ready().await,
                WorkerCommand::Stop => supervisor("stop").await,
                WorkerCommand::Deploy => {
                    let mut data = Vec::new();
                    std::io::stdin().take(1 << 20).read_to_end(&mut data)?;
                    worker_deploy(serde_json::from_slice(&data)?).await
                }
            },
        }
    }
    async fn deploy(&mut self, options: Deploy) -> Result<()> {
        if let Some(v) = options.min_vram {
            self.config.min_vram_gb = v;
        }
        if let Some(v) = options.context {
            self.config.context_tokens = v;
        }
        if let Some(v) = options.local_port {
            self.config.local_port = v;
        }
        if let Some(v) = options.max_price {
            self.config.max_price = v;
        }
        let mut d = Deployment {
            model: if options.name.is_empty() {
                default_model_name(&options.target)
            } else {
                options.name
            },
            engine: options.engine,
            target: options.target,
            hf: options.hf,
            tokenizer: options.tokenizer,
            args: options.args,
            modelfile: options.modelfile,
            context: self.config.context_tokens,
            ..Deployment::default()
        };
        if preset(&d.target).is_none() && !d.hf {
            d.target = model_path(&d.target)?;
        }
        if !d.modelfile.is_empty() {
            d.modelfile = model_path(&d.modelfile)?;
        }
        if ["/", "home:", "vol:"]
            .iter()
            .any(|p| d.tokenizer.starts_with(p))
        {
            d.tokenizer = model_path(&d.tokenizer)?;
        }
        validate_deployment(&d)?;
        let pod = self.up("", "small", false, true).await?;
        let version = self.remote_output(&pod, &["fastgpu", "--version"]).await?;
        if version.trim() != format!("fastgpu version {VERSION}") {
            bail!(
                "remote CLI version mismatch ({}): use an image built with fastgpu {VERSION}; terminate the old Pod first with fastgpu down",
                version.trim()
            );
        }
        self.remote(
            &pod,
            &["fastgpu", "worker", "deploy"],
            Some(&serde_json::to_vec(&d)?),
            false,
            false,
            false,
        )
        .await?;
        tokio::time::timeout(Duration::from_secs(20 * 60), async {
            loop {
                if self
                    .remote_output(&pod, &["fastgpu", "worker", "ready"])
                    .await
                    .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        })
        .await
        .context("model not ready: check fastgpu logs; the Pod remains running")?;
        self.tunnel(&pod, self.config.local_port).await?;
        ready_model(&format!("http://127.0.0.1:{}", self.config.local_port), &d).await?;
        println!(
            "Engine: {}\nModel: {}\nBase URL: http://127.0.0.1:{}/v1",
            d.engine, d.model, self.config.local_port
        );
        Ok(())
    }
    async fn volumes(&mut self, command: VolumeCommand) -> Result<()> {
        if !matches!(command, VolumeCommand::List) {
            self.acquire()?;
        }
        let api = self.client()?;
        let selector = match &command {
            VolumeCommand::List => return print_json(&api.volumes().await?),
            VolumeCommand::Create {
                name,
                size,
                datacenter,
                tier,
            } => {
                if !(10..=4096).contains(size)
                    || datacenter.is_empty()
                    || !["STANDARD", "HIGH_PERFORMANCE"].contains(&tier.as_str())
                {
                    bail!(
                        "specify --datacenter, --size between 10 and 4096, and --type STANDARD or HIGH_PERFORMANCE"
                    );
                }
                let value = api
                    .request(
                        Method::POST,
                        "/network-volumes",
                        Some(&json!({"name":name,"size":size,"dataCenter":datacenter,"type":tier})),
                    )
                    .await?;
                return print_json(&serde_json::from_value::<Volume>(value)?);
            }
            VolumeCommand::Show { selector }
            | VolumeCommand::Use { selector }
            | VolumeCommand::Rename { selector, .. }
            | VolumeCommand::Resize { selector, .. }
            | VolumeCommand::Delete { selector, .. } => selector,
        };
        let volume = resolve_volume(&api.volumes().await?, selector)?;
        let path = format!("/network-volumes/{}", escape(&volume.id));
        let patch = match command {
            VolumeCommand::Show { .. } => return print_json(&volume),
            VolumeCommand::Use { .. } => {
                if !self.state.pod_id.is_empty() && self.state.volume_id != volume.id {
                    bail!("the selected Pod uses another volume; run fastgpu down first");
                }
                self.config.volume = volume.id;
                return self.save_config();
            }
            VolumeCommand::Rename { new_name, .. } => {
                if new_name.trim().is_empty() {
                    bail!("empty name");
                }
                json!({"name":new_name})
            }
            VolumeCommand::Resize { gb, .. } => {
                if gb < volume.size || gb > 4096 {
                    bail!(
                        "size must be between {} and 4096 GB; shrinking is not allowed",
                        volume.size
                    );
                }
                json!({"size":gb})
            }
            VolumeCommand::Delete { confirm, .. } => {
                if confirm != volume.id {
                    bail!("irreversible operation: specify --confirm {}", volume.id);
                }
                if volume.id == self.config.volume || volume.id == self.state.volume_id {
                    bail!("protected volume: selected by the profile or managed Pod");
                }
                for pod in api.pods().await? {
                    if mounted(&pod, &volume.id) {
                        bail!("volume attached to Pod {}; deletion refused", pod.id);
                    }
                }
                api.request(Method::DELETE, &path, None).await?;
                return Ok(());
            }
            _ => unreachable!(),
        };
        print_json(&serde_json::from_value::<Volume>(
            api.request(Method::PATCH, &path, Some(&patch)).await?,
        )?)
    }
}

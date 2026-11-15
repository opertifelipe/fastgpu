use crate::{
    api::{Volume, escape, resolve_volume},
    config::{App, State, WORKSPACE, print_json},
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

const HELP_BANNER: &str = r#"  __           _
 / _| __ _ ___| |_ __ _ _ __  _   _
| |_ / _` / __| __/ _` | '_ \| | | |
|  _| (_| \__ \ || (_| | |_) | |_| |
|_|  \__,_|___/\__\__, | .__/ \__,_|
                  |___/|_|

      Created by Felipe Operti
"#;

#[derive(Parser)]
#[command(name = "fastgpu", before_help = HELP_BANNER, about = "Your remote GPU environment on RunPod", version = concat!("version ", env!("CARGO_PKG_VERSION")))]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,
}
#[derive(Subcommand)]
pub enum Commands {
    /// Configure the image, SSH key, and volume
    ///
    /// Save the local profile used by future commands. Select an existing RunPod
    /// Network Volume by ID or unique name; when several exist, prompt in a terminal.
    /// Generate an Ed25519 key if the selected key does not exist. Requires RunPod
    /// credentials, but does not create a Pod or a volume. Image changes apply to
    /// newly created Pods; the image must contain the same fastgpu version as the client.
    ///
    /// Example: fastgpu init --volume VOLUME_ID --image YOUR_ACCOUNT/fastgpu-workspace:0.3.0
    Init {
        #[arg(long, help = "Container image tag or digest for new Pods")]
        image: Option<String>,
        #[arg(
            long,
            help = "Local private SSH key path; its .pub file must also exist"
        )]
        ssh_key: Option<String>,
        #[arg(long, help = "RunPod registry credential ID for a private image")]
        registry: Option<String>,
        #[arg(
            long,
            default_value = "",
            help = "Existing Network Volume ID or unique name"
        )]
        volume: String,
    },
    /// Show configuration without credentials
    ///
    /// Print the effective local configuration as JSON, including the image,
    /// volume selection, SSH key path, hardware requirements, context, and port.
    /// Reads the profile and applies defaults without contacting RunPod or writing
    /// files. Does not print the API key or private SSH key contents.
    Config,
    /// Create or resume one Pod (default size: small)
    ///
    /// Reuse the selected Pod, starting it if stopped, or create one paid On-Demand
    /// GPU Pod in the selected Network Volume's data center. Wait for SSH and,
    /// unless --skip-model is set, download and verify the default Qwen GGUF.
    /// Does not start inference or copy local project files. An existing Pod keeps
    /// its GPU and image; run down before replacing it. Flags affect this invocation.
    ///
    /// Examples: fastgpu up --small --dry-run; fastgpu up --small --skip-model
    Up(Up),
    /// List Pods without modifying them
    ///
    /// Query RunPod and print all account Pods as JSON, including Pods outside
    /// this profile. Useful for finding an existing Pod after losing local state
    /// or an interrupted creation. Does not select, start, or stop any Pod.
    Pods,
    /// Show Pod details, status, and compute rate
    ///
    /// Print the selected Pod's current API details together with local state.
    /// Without a selected Pod, print local state only and do not contact RunPod.
    /// Use info for a broader hardware/storage overview, or logs for diagnostics.
    Status,
    /// Select a Pod with the fastgpu image and a Network Volume
    ///
    /// Adopt an existing gpu-* Pod into this profile after checking its Network
    /// Volume attachment and fastgpu executable over SSH. The Pod must be reachable.
    /// Saves its ID and volume locally, making it the target of shell, deploy,
    /// stop, and down. Does not create or restart a Pod.
    Use {
        /// Existing Pod ID from fastgpu pods
        id: String,
    },
    /// Terminate the managed Pod and retain the Network Volume
    ///
    /// Close the managed SSH tunnel and permanently delete the selected managed
    /// Pod. Its container disk is lost; files on /workspace and the persistent home
    /// remain on the Network Volume. Verify the volume still exists before clearing
    /// local state. Storage billing continues. Use stop to keep the Pod instead.
    Down,
    /// Stop the Pod (volume billing continues)
    ///
    /// Close the managed tunnel and ask RunPod to stop the selected managed Pod.
    /// Keep its ID and storage so up can attempt a restart. GPU availability is
    /// not reserved while stopped. RunPod must allow the stop action in the Pod's
    /// current state. Use down when you want to delete compute while retaining the volume.
    Stop,
    /// Clear an uncertain creation after manual verification
    ///
    /// Without --clear, print local state without contacting RunPod. A pending_name
    /// records a Pod creation whose API response was lost, so up cannot safely
    /// create another. Check fastgpu pods and the RunPod console first. If the Pod
    /// exists, select it with fastgpu use ID. Otherwise --clear checks the API for
    /// that operation name and resets local state. Refuses while a Pod is selected;
    /// does not delete Pods, volumes, or files.
    Recover {
        #[arg(
            long,
            help = "Confirm that you also checked the console for the absent Pod"
        )]
        clear: bool,
    },
    /// Open an interactive shell or run a remote command
    ///
    /// Connect to the selected Pod as gpu over SSH. With no command, open an
    /// interactive login shell; otherwise execute the given program and return
    /// its exit status. The default working directory is the persistent home.
    /// Use --cwd home:PATH or vol:PATH to change it. For pipes, redirects, or shell
    /// expansion, explicitly invoke bash -lc with a quoted command string.
    ///
    /// Examples: fastgpu shell mkdir -p projects/demo; fastgpu shell python train.py
    Shell {
        #[arg(
            long,
            default_value = "home:",
            help = "Remote working directory (home:PATH or vol:PATH)"
        )]
        cwd: String,
        #[arg(long, help = "Allocate a terminal even when running a command")]
        tty: bool,
        /// Remote executable followed by its arguments; omit for an interactive shell
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
    /// Open a remote directory in local VS Code
    ///
    /// Launch local VS Code with Remote - SSH against the selected Pod. The remote
    /// directory must already exist; omit it to open the persistent home. Add an
    /// Include for fastgpu's SSH configuration to ~/.ssh/config while preserving
    /// existing entries. Requires local code and the Remote - SSH extension.
    ///
    /// Example: fastgpu code home:projects/demo
    Code {
        /// Existing remote directory, relative to home or prefixed with home:/vol:
        directory: Option<String>,
    },
    /// Copy local files to the Pod or its persistent Network Volume
    ///
    /// Upload SOURCE with rsync over SSH into remote DESTINATION, creating that
    /// directory if needed. home:PATH (the default) and vol:PATH store data on the
    /// Network Volume; an absolute path such as /tmp/demo uses the Pod filesystem.
    /// Files outside the volume are lost when the Pod is deleted. A source directory
    /// ending in / copies its contents; otherwise copies the directory itself.
    /// Existing same-name files may be updated, but destination-only files remain.
    /// Excludes .venv/, __pycache__/, .env, .env.*, *.pem, and *.key. This is a
    /// one-time copy, not continuous synchronization; use pull for the reverse.
    ///
    /// Examples: fastgpu push ./data/ vol:datasets; fastgpu push ./input.txt /tmp/job
    Push {
        /// Local file or directory to upload
        source: String,
        /// Remote destination directory: home:PATH, vol:PATH, or an absolute path
        destination: String,
    },
    /// Copy files from the Pod or Network Volume to this computer
    ///
    /// Download remote SOURCE with rsync over SSH into local DESTINATION, creating
    /// that directory if needed. Use home:PATH, vol:PATH, or an absolute Pod path.
    /// A source directory ending in / copies its contents; otherwise copies the
    /// directory itself. Existing same-name files may be updated, but destination-only
    /// files remain. Excludes .venv/, __pycache__/, .env, .env.*, *.pem, and *.key.
    /// Requires a reachable selected Pod even when reading the Network Volume.
    /// Does not remove the remote files; use push for the reverse direction.
    ///
    /// Examples: fastgpu pull vol:outputs/ ./results; fastgpu pull /tmp/job/log.txt ./logs
    Pull {
        /// Remote source file or directory: home:PATH, vol:PATH, or an absolute path
        source: String,
        /// Local destination directory
        destination: String,
    },
    /// Manage persistent GGUF models
    ///
    /// Download a supported preset with model pull, or inspect installed GGUF
    /// files with model list. Both commands run on the selected Pod and use the
    /// Network Volume; neither starts inference. With no subcommand, show help.
    Model {
        #[command(subcommand)]
        command: Option<ModelCommand>,
    },
    /// Serve a GGUF or checkpoint with llama.cpp, Ollama, or vLLM
    ///
    /// Create/reuse a small Pod, prepare the requested model, start one inference
    /// service, wait until ready, and open an SSH tunnel to the local API port.
    /// Defaults to qwen3.8-27b with llama.cpp. Presets are downloaded if missing;
    /// custom paths refer to files already on the remote Network Volume. vLLM also
    /// accepts a Hugging Face ID with --hf. Changing the deployment replaces the
    /// previous inference service; an identical ready deployment is reused.
    /// The image worker must match this client's version. Failure leaves the Pod
    /// running for diagnosis: check logs, then down to release compute.
    ///
    /// Examples: fastgpu deploy qwen3.8-27b; fastgpu deploy vol:models/custom.gguf --context 4096
    Deploy(Deploy),
    /// Stop inference; the Pod remains running
    ///
    /// Close the local managed tunnel and stop the remote inference service.
    /// Keep the deployment manifest, downloaded models, and all volume data.
    /// Compute charges continue while the Pod runs; use down to release it.
    Undeploy,
    /// Open or close the model tunnel
    ///
    /// Forward 127.0.0.1:LOCAL_PORT to port 8080 on the selected Pod over SSH.
    /// Defaults to the profile port (8080). Requires a reachable Pod; does not start
    /// inference. Reuses an existing tunnel on the same port. Close it before
    /// switching ports. --close closes only fastgpu's managed tunnel and keeps
    /// the Pod and inference service running.
    Tunnel {
        #[arg(
            long,
            help = "Local loopback port for the API (default: profile local_port)",
            conflicts_with = "close"
        )]
        local_port: Option<i64>,
        #[arg(long, help = "Close the managed SSH tunnel")]
        close: bool,
    },
    /// Show model or container logs
    ///
    /// Print the last 100 lines of the remote inference log through SSH. --follow
    /// keeps reading new lines until interrupted. --pod instead reads RunPod's
    /// container log stream through the API, useful before SSH is available;
    /// the stream may stay open until interrupted. Neither option changes resources.
    Logs {
        #[arg(
            long,
            help = "Follow inference log updates until Ctrl-C",
            conflicts_with = "pod"
        )]
        follow: bool,
        #[arg(
            long,
            help = "Read the provider's container log stream instead of inference logs"
        )]
        pod: bool,
    },
    /// Check dependencies and remote installations
    ///
    /// Check local ssh and rsync, report optional VS Code, and verify RunPod API
    /// access by listing volumes. With --remote, also run the image's installation
    /// checks over SSH, including GPU/CUDA and inference tools.
    /// Does not create or resume a Pod. A successful check does not guarantee that
    /// a particular model or context length fits the GPU.
    Doctor {
        #[arg(long, help = "Also inspect the selected Pod over SSH")]
        remote: bool,
    },
    /// Show the compute rate and creation time (not a bill)
    ///
    /// Print the selected Pod ID, current status, hourly compute rate, and creation
    /// timestamp from RunPod. Does not calculate accrued charges: creation time
    /// includes stopped periods, and storage is billed separately. Consult RunPod
    /// billing for actual spend.
    Cost,
    /// View and manage Network Volumes
    ///
    /// List account volumes by default, or inspect, select, create, rename, enlarge,
    /// or delete one with a subcommand. Volumes survive Pod deletion and incur
    /// storage charges independently. Mutations are explicit; delete requires an
    /// exact ID confirmation and refuses selected or attached volumes. The singular
    /// alias volume accepts the same commands.
    #[command(alias = "volume")]
    Volumes {
        #[command(subcommand)]
        command: Option<VolumeCommand>,
    },
    /// Show hardware, storage, status, networking, and connection commands
    ///
    /// Print a JSON overview of the selected Pod using the RunPod API, including
    /// hardware, volume attachment, and connection information. --live adds SSH
    /// probes for GPU, system, and persistent-home details. Omits credential-bearing
    /// environment/registry fields. Use connect for only the connection commands.
    Info {
        #[arg(long, help = "Also probe the running Pod over SSH")]
        live: bool,
    },
    /// Print the TCP endpoint, SSH command, and tunnel command
    ///
    /// Generate copyable commands for direct SSH and TCP forwarding to the selected
    /// Pod, plus the local endpoint. Refresh fastgpu's SSH configuration using its
    /// current public port mapping. Does not open a shell or tunnel. --remote-port
    /// selects the service port on the Pod; --local-port selects its local endpoint.
    /// Use --json for structured output.
    Connect {
        #[arg(long, help = "Print connection details as JSON")]
        json: bool,
        #[arg(long, help = "Local forwarding port (default: profile local_port)")]
        local_port: Option<i64>,
        #[arg(
            long,
            default_value_t = 8080,
            help = "Remote loopback service port to forward"
        )]
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
    #[arg(
        long,
        help = "Maximum compute price in USD/hour; storage is billed separately"
    )]
    max_price: Option<f64>,
    #[arg(long, help = "Minimum GPU VRAM in GB (default: profile min_vram_gb)")]
    min_vram: Option<i64>,
    #[arg(
        long,
        help = "Container image for a new Pod; does not replace an existing Pod"
    )]
    image: Option<String>,
    #[arg(
        long,
        help = "Ephemeral container disk in GB, at least 20 (default: profile setting)"
    )]
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
    /// Preset alias, persistent remote model path, or Hugging Face ID with --hf
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
    #[arg(
        long,
        default_value = "",
        help = "vLLM tokenizer Hugging Face ID or persistent path; required for GGUF"
    )]
    tokenizer: String,
    #[arg(
        long,
        default_value = "",
        help = "Persistent remote Ollama Modelfile to use for import"
    )]
    modelfile: String,
    #[arg(
        long = "arg",
        help = "Additional engine argument; repeatable, e.g. --arg=--enforce-eager"
    )]
    args: Vec<String>,
    #[arg(long, help = "Minimum GPU VRAM in GB when creating a Pod")]
    min_vram: Option<i64>,
    #[arg(
        long,
        help = "Context length in tokens, 512..262144 (default: profile setting); must fit GPU memory"
    )]
    context: Option<i64>,
    #[arg(
        long,
        help = "Local loopback port for the model API (default: profile local_port)"
    )]
    local_port: Option<i64>,
    #[arg(long, help = "Maximum compute price in USD/hour when creating a Pod")]
    max_price: Option<f64>,
}
#[derive(Subcommand)]
pub enum ModelCommand {
    /// Download and verify a preset: nanbeige4.3-33b, qwen3.8-27b
    ///
    /// Download the pinned GGUF directly onto the selected Pod's Network Volume
    /// at /workspace/.gpu/models. Resume partial downloads and verify file size
    /// and SHA-256 before use. Reuse an already verified file. Defaults to Qwen
    /// (about 16.5 GB); Nanbeige uses about 20.8 GB. Requires a matching worker and
    /// enough free volume space. Does not start a Pod or inference, and does not
    /// download to this computer (use fastgpu pull for that).
    Pull {
        /// Supported preset alias to download
        #[arg(default_value = MODEL_ALIAS)]
        model: String,
    },
    /// List installed GGUFs with their deploy alias or path
    ///
    /// Recursively scan /workspace/.gpu/models on the selected Pod, or a supplied
    /// persistent directory. Show each complete .gguf file's size and the alias
    /// or quoted path to pass to deploy. Ignore partial downloads and symlinks.
    /// Listing does not download, verify checksums, or start inference.
    ///
    /// Examples: fastgpu model list; fastgpu model list vol:models
    List {
        /// Persistent remote directory (default: /workspace/.gpu/models)
        directory: Option<String>,
    },
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
    ///
    /// Query all account Network Volumes and print their IDs, names, sizes, and
    /// data centers as JSON. Does not select or change storage. This is also the
    /// default action of fastgpu volumes.
    List,
    /// Show volume details
    ///
    /// Find one existing Network Volume by ID or unique name and print its API
    /// details as JSON. Ambiguous names are rejected; use its ID instead. Does
    /// not require a selected or running Pod.
    Show {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
    },
    /// Select the profile volume
    ///
    /// Save an existing volume's ID in the local profile for future Pod creation.
    /// Does not move data or remount a running Pod. Refuse when the selected Pod
    /// uses another volume; terminate it with down before switching volumes.
    Use {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
    },
    /// Rename a volume
    ///
    /// Change the selected volume's display name through RunPod. Its ID, data,
    /// size, and Pod attachments remain the same. Use a nonempty name; unique
    /// names make later selection easier. This changes the account resource.
    Rename {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
        new_name: String,
    },
    /// Increase volume size
    ///
    /// Set an existing volume's capacity in GB through RunPod. The new size must
    /// be at least its current size and no more than 4096 GB; shrinking is refused.
    /// Existing files remain, but larger capacity increases ongoing storage cost.
    /// This is a persistent account change, independent of the selected Pod.
    Resize {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
        /// New total capacity in GB, not the amount to add
        gb: i64,
    },
    /// Delete an unprotected volume with ID confirmation
    ///
    /// Permanently delete a Network Volume and its files through RunPod. Requires
    /// --confirm with the exact volume ID, even when selecting by name. Refuse
    /// the configured volume, the managed Pod's volume, and any volume attached
    /// to an account Pod. Use down to delete only compute while retaining data.
    Delete {
        #[arg(value_name = "ID_OR_NAME")]
        selector: String,
        #[arg(
            long,
            default_value = "",
            help = "Exact volume ID confirming permanent deletion of its data"
        )]
        confirm: String,
    },
    /// Create a Network Volume (paid storage)
    ///
    /// Allocate persistent storage in the explicitly selected RunPod data center.
    /// Requires a name, --size between 10 and 4096 GB, and --datacenter. STANDARD
    /// is the default tier; HIGH_PERFORMANCE depends on provider availability.
    /// Print the created volume as JSON. Does not select it or create a Pod;
    /// use volumes use afterward. Storage charges begin independently of compute.
    Create {
        name: String,
        #[arg(
            long,
            default_value_t = 0,
            help = "Initial capacity in GB, 10..4096; required for creation"
        )]
        size: i64,
        #[arg(
            long,
            default_value = "",
            help = "RunPod data center ID; constrains future GPU placement"
        )]
        datacenter: String,
        #[arg(
            long = "type",
            default_value = "STANDARD",
            help = "Storage tier: STANDARD or HIGH_PERFORMANCE"
        )]
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
    let Some(command) = cli.command else {
        Cli::command().print_help()?;
        println!();
        return Ok(());
    };
    App::new()?.dispatch(command).await
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
                self.check_worker(&pod).await?;
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
                self.check_worker(&pod).await?;
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
                    let pod = self.selected(&api).await?;
                    self.check_worker(&pod).await?;
                    self.remote(
                        &pod,
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
        self.check_worker(&pod).await?;
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

# Manuale completo della CLI fastgpu

Questa guida descrive la versione 0.3.0. I comandi si eseguono sul computer locale, salvo indicazione diversa. I blocchi seguenti conservano il riferimento tecnico originale in inglese. Per imparare a costruire il progetto, vedi [Lezioni di Rust](LEZIONI_RUST.md).


## Installazione e configurazione


## Local requirements

- Linux x86-64 for the supplied GPU image. The Rust client can be built for other Linux architectures, but these have not all been tested.
- OpenSSH client, `ssh-keygen`, and rsync.
- Rust 1.91 or later with Cargo, a C toolchain, and make to build from source.
- VS Code with Remote - SSH if you use `fastgpu code`.
- A RunPod account, available credit, and access to the Pod and Network Volume APIs.

On Ubuntu/Debian, install the system tools with:

```sh
sudo apt-get update
sudo apt-get install -y openssh-client rsync make build-essential
```

Install Rust and Cargo using the [official instructions](https://www.rust-lang.org/tools/install). Install the development checks with `rustup component add rustfmt clippy`. A C compiler and linker are needed for the build, including the TLS cryptography dependency. The compiled client needs no Rust runtime or OpenSSL installation; HTTPS uses rustls. Docker is needed to rebuild the image, not to use the client.

## Build and install

From the project directory:

```sh
make install
export PATH="$HOME/.local/bin:$PATH"
fastgpu --version
fastgpu --help
```

The binary is installed in `~/.local/bin/fastgpu`. To select Cargo, use `make install CARGO=/path/to/cargo`. To build only `bin/fastgpu`, run `make build`; this runs `cargo build --release --locked`. For a debug build, use `cargo build --locked`. Run `make test` for formatting, Rust tests, and Clippy. Add the PATH export to your shell configuration.

You need a local copy of the source. Publishing it on GitHub is not required to build or use the CLI.

## Install a local npm package

On Linux x64 with glibc, use Node.js 22.15+ in the 22.x series or Node.js 24+ and npm:

```sh
make npm-package
npm install -g ./dist/fastgpu-0.3.0.tgz
fastgpu --version
```

`make npm-package` builds and packages the Rust executable in `dist/`. If you already have the `.tgz`, only the npm installation step is needed; it does not compile or download a binary. OpenSSH and rsync remain system dependencies. No registry publication is needed, and `package.json` is marked private.

The executable is installed under npm's global prefix, which you can inspect with `npm prefix -g`. Use `command -v fastgpu` to check which installation your shell selects if you previously installed with `make install`. See [npm packaging](CLI.md) for binary compatibility and package validation.

## RunPod API key

Generate an API key in your RunPod account settings. Grant access to the operations you need: reading the catalog, Pods, and volumes; creating and managing Pods; and volume write permissions if you plan to create or modify storage.

The client checks, in order:

1. `RUNPOD_API_KEY` in the local environment;
2. `apikey` in `~/.runpod/config.toml`, if runpodctl is configured.

```sh
export RUNPOD_API_KEY='YOUR_RUNPOD_API_KEY'
fastgpu volumes list
```

Replace the placeholder. To avoid putting the key in shell history, enter it interactively in Bash:

```bash
read -rs -p 'RunPod API key: ' RUNPOD_API_KEY
printf '\n'
export RUNPOD_API_KEY
```

The API key stays on your local computer. It is not saved in `config.json`, printed by help, or sent to the Pod. Keep it out of source files and files copied to the remote project. Protect the fallback TOML file because it contains the secret.

## Volume and image

```sh
fastgpu volumes list
fastgpu init --image YOUR_ACCOUNT/fastgpu-workspace:0.3.0
fastgpu up --dry-run
```

When several volumes are available, the first `init` asks which one to select. The saved selection is reused. Use an ID if names are duplicated.

To select these explicitly:

```sh
fastgpu init --volume ID_OR_NAME --image YOUR_ACCOUNT/fastgpu-workspace:0.3.0
```

The default image reference is `felipeoperti/fastgpu-workspace:0.3.0`. This migration prepares its Rust build but does not publish the image. [Build and publish your own matching image](CLI.md), replace `YOUR_ACCOUNT` in the examples, and select it with `--image`. Existing images from 0.2.1 contain the previous client and do not meet the 0.3.0 deployment version check.

The image includes Ubuntu/CUDA, Python, uv, vLLM, llama.cpp, Ollama, GitHub CLI, Codex CLI, SSH, rsync, and tmux. For a private image, register a credential in RunPod and pass `--registry CREDENTIAL_ID`; the client stores only its ID.

Storage must be a **Network Volume** in the same data center as the Pod. A container disk or Pod volume disk does not provide the persistence after termination required by this project.

## SSH key and direct access

`fastgpu init` generates a dedicated Ed25519 key pair, normally at:

```text
~/.config/gpu/id_ed25519
~/.config/gpu/id_ed25519.pub
```

The generated key has no passphrase. To use an existing key instead:

```sh
fastgpu init --ssh-key ~/.ssh/id_ed25519
```

Both the private key and its `.pub` file must exist. Load passphrase-protected keys into `ssh-agent` with `ssh-add` before running commands: automatic operations use SSH in batch mode. The agent is not forwarded to the Pod.

During `up`, only the **public** key is sent to the image. The server allows the **`gpu`** user, exposes `22/tcp`, and uses key authentication. The CLI queries RunPod for the assigned IP and public port; the external port can change after a restart.

## SSH configuration and VS Code

With VS Code installed locally, install the extension:

```sh
code --install-extension ms-vscode-remote.remote-ssh
```

The CLI generates `~/.config/gpu/ssh_config` with an alias named `gpu-POD_ID`, the local private key, the Pod's IP/port, and a dedicated `known_hosts` file. To get commands ready to copy:

```sh
fastgpu connect
fastgpu connect --json
fastgpu shell
```

`fastgpu code DIRECTORY` adds a line like this at the beginning of `~/.ssh/config` if it is missing:

```sshconfig
Include "/home/USER/.config/gpu/ssh_config"
```

The actual path is generated automatically and existing SSH configuration is preserved. Other commands use the dedicated file directly through `ssh -F`, so they do not require manual edits to the global SSH configuration.

The RunPod dashboard may show SSH commands using `root`. For this image, use `fastgpu connect`, which selects `gpu`. Transfers and tunnels use direct SSH over TCP. The provider's SSH proxy has different capabilities and is not the CLI's transport.

The first connection records the host key. Unexpected key changes for the same Pod are rejected; verify the server's identity before editing `known_hosts`.

## Persistent local configuration

The profile directory is `$XDG_CONFIG_HOME/gpu`, normally `~/.config/gpu`. To use an independent profile:

```sh
export GPU_CONFIG_DIR="$HOME/.config/gpu-experiments"
fastgpu init
```

The Rust migration retains `GPU_CONFIG_DIR`, the `gpu` profile directory, `gpu-*` Pod names and SSH aliases, the remote `gpu` user, and `/workspace/.gpu`. Existing profiles, keys, Pod references, and volume contents remain usable without a path migration. The executable name is `fastgpu` both locally and inside the image.

| File | Purpose |
| --- | --- |
| `config.json` | Image, volume, minimum resources, SSH key path, and model settings |
| `state.json` | Managed Pod ID, volume, and tunnel state |
| `id_ed25519`, `.pub` | Dedicated SSH identity |
| `ssh_config`, `known_hosts` | Connection settings and server identities |
| `tunnel-*`, `lock` | Sockets and protection against concurrent changes |

`fastgpu config` displays the settings. You can edit the JSON while no operation is running. Defaults are 24 GB VRAM, 32 GB system RAM, host CUDA 13.0 or newer, a 40 GB container disk, a 131,072-token context, and local port 8080. There is no price cap until you set `max_price` or pass `--max-price`.

`up` and `deploy` flags apply to the current invocation; `init` saves the settings it configures. Keep profile paths short enough for SSH Unix socket limits.

## Upgrade the client and image

Rebuild with `make install`, then check that `fastgpu --version` prints `fastgpu version 0.3.0`. This installs `~/.local/bin/fastgpu`. Existing configuration and state files can be read directly by the Rust client; no profile conversion is required.

Build and publish a 0.3.0 image first. Save your work on the volume, terminate the managed Pod with `fastgpu down`, select your image with `fastgpu init --image YOUR_ACCOUNT/fastgpu-workspace:0.3.0`, and recreate the environment with `fastgpu up --skip-model`. A saved profile keeps its image reference until you change it explicitly. Updating the local binary alone does not change the remote image. The inference engines, model weights, and persistent paths retain their existing formats.

New deployments require `fastgpu` inside the image with the same version as the client. This prevents new commands from being sent to an older image. Use a Docker digest rather than a mutable tag to select an exact image build.


## Riferimento dei comandi


This reference describes the Rust CLI (0.3.0). Commands, flags, profile JSON, and persistent paths are retained from the previous implementation. Run `cargo run --locked -- --help` from source, or `fastgpu --help` after installation.

## Help and version

```sh
fastgpu --help
fastgpu help deploy
fastgpu deploy --help
fastgpu volumes --help
fastgpu --version
```

Built-in help is available without an API key and lists options, arguments, and command descriptions. Command and option names match the examples below.

## Configuration

| Command | Purpose |
| --- | --- |
| `fastgpu init` | Select the volume, prepare the SSH identity, and save the profile |
| `fastgpu init --volume ID_OR_NAME` | Select a Network Volume explicitly |
| `fastgpu init --image IMAGE` | Set the image for new Pods |
| `fastgpu init --ssh-key PATH` | Use an existing local key with its `.pub` file |
| `fastgpu init --registry ID` | Select a RunPod registry credential for a private image |
| `fastgpu config` | Show settings without the API key |

The default profile is `~/.config/gpu`; override it with `XDG_CONFIG_HOME` or `GPU_CONFIG_DIR`. API credentials come from `RUNPOD_API_KEY` or runpodctl's TOML configuration. See [installation](CLI.md) for the full procedure.

## Pods and costs

| Command | Effect |
| --- | --- |
| `fastgpu` or `fastgpu up` | Create/resume the Pod (default size: small) and prepare the Qwen preset on the volume |
| `fastgpu up --small` | Select the cheapest compatible GPU below $1.50/hour |
| `fastgpu up --medium` | Select the GPU closest to $3/hour, from $1.50 to below $3.50/hour |
| `fastgpu up --large` | Select the cheapest compatible GPU from $3.50 to $5/hour |
| `fastgpu up --xlarge` | Select the cheapest compatible GPU above $5/hour |
| `fastgpu up --skip-model` | Start the environment without preparing Qwen |
| `fastgpu up --dry-run` | Show the plan without allocating resources |
| `fastgpu up --max-price 0.90` | Cap compute cost in USD/hour |
| `fastgpu up --min-vram 48` | Set minimum VRAM for a new Pod |
| `fastgpu up --image IMAGE --disk 50` | Set the image and container disk for a new Pod |
| `fastgpu pods` | List account Pods without changing them |
| `fastgpu use ID` | Recover a `gpu-*` Pod with a Network Volume and a verifiable image |
| `fastgpu status` | Show remote status and local references |
| `fastgpu cost` | Show the compute rate and creation time, not the total bill |
| `fastgpu stop` | Stop the Pod while retaining its identity |
| `fastgpu down` | Terminate the managed Pod and verify that its volume still exists |

Selection uses Secure Cloud, one NVIDIA GPU, On-Demand billing, and the volume's data center. Catalog pricing is an estimate; the actual rate is checked after creation. If it exceeds `--max-price`, the newly created Pod is terminated and the volume retained. Spot instances are not implemented.

Size flags are mutually exclusive. Selection and capacity retries stay within the chosen price range and any configured `--max-price`; no matching offer causes an error. Medium ranks offers by distance from $3/hour, preferring the cheaper offer on a tie. Other sizes rank by increasing price.

Repeating `up` reports that the Pod already exists and reuses it. An existing Pod keeps its GPU, image, and disk. Run `fastgpu down` before creating a new machine or changing size, keeping your data on the Network Volume. When no Pod is selected locally, creation checks all account Pods and refuses if another non-terminated `gpu-*` Pod exists, including a stopped Pod. Recover that Pod with `fastgpu use ID` when SSH is available, or run `down` from its original profile.

Stopping does not reserve the GPU. Resuming may fail because the host has no free capacity. The CLI does not automatically turn that error into termination. `down` followed by `up` allows a new machine to be selected.

## Machine information

```sh
fastgpu info
fastgpu info --live
fastgpu connect
fastgpu connect --json
fastgpu connect --local-port 8888 --remote-port 8888
```

`info` shows operational API fields: identity and status, image, cloud/data center, host CUDA, assigned GPU/CPU resources, RAM, disks and mounts, ports, SSH details, runtime metrics, rate, and timestamps. It also includes volume details and persistent paths. Environment variables, startup arguments, and registry credentials are omitted because they may contain secrets.

CLI help, prompts, messages, and errors are in English. `info` uses the JSON fields `volumes`, `connections`, `persistence`, `note`, and, with `--live`, `remote_system`. Connection output uses `status`, `local_endpoint`, and `local_api` alongside `tcp`, `ssh`, `ssh_config`, and `tunnel_tcp`. Scripts using the former Italian field names must update their lookups.

RunPod's `gpu.memory` field is assigned **system RAM**, not VRAM. `--live` reads the GPU name, total/used VRAM, utilization, driver, operating system, and resolved home directory over SSH. Visible CPU counts and kernel memory can describe the host; use API fields for Pod allocations.

`connect` prints an `IP:PORT` endpoint, an SSH command with the dedicated identity, a command using `ssh -F`, and a TCP forwarding example. It also reports the local endpoint and, for the managed inference port, the API URL. It does not connect. Tunnel ports are configurable; you must start the remote service listening on the requested port. If the Pod is stopped or its mapping is not ready, no usable SSH command is printed.

## Interactive shell and remote execution

```sh
fastgpu shell
fastgpu shell python --version
fastgpu shell python train.py
fastgpu shell --cwd home:projects/experiment python train.py
fastgpu shell --tty tmux new-session -A -s training
fastgpu shell -- bash -lc 'pwd && nvidia-smi'
```

Without a program, `shell` opens interactive SSH in the persistent home directory. `exit` closes the session. With a program, stdout, stderr, and the exit code are preserved. The program and its files must exist on the Pod.

Place `fastgpu shell` options before the program: subsequent arguments belong to the remote program. `--tty` allocates a terminal for interactive applications. For pipelines, variables, and redirection, explicitly use `bash -lc` with quoting; unquoted operators are interpreted by your local shell first.

## Files and VS Code

```sh
fastgpu mkdir projects/demo
fastgpu code projects/demo
fastgpu code
fastgpu push ./demo/ home:projects/demo
fastgpu push ./my-model.gguf vol:models
fastgpu pull home:checkpoints/ ./outputs
```

Without a path, `code` opens the remote home directory (`/home/gpu`) directly. With a path, it opens that folder directly. VS Code requires Remote - SSH; the CLI prepares an alias and adds an `Include` to `~/.ssh/config` if needed.

For `mkdir`, `push`, `pull`, `code`, and `shell --cwd`, paths are relative to the home directory or use `home:`/`vol:`. Model deployments also accept absolute paths; use prefixes for transfers. Traversal beyond the selected root and newlines are rejected. Symlinks follow normal filesystem rules: these prefixes are not a sandbox.

Transfer destinations are directories. Rsync preserves trailing-slash semantics on the source. It keeps destination-only files but may update existing ones. It excludes `.venv`, `__pycache__`, `.env`, `.env.*`, `*.pem`, and `*.key`; other secrets are not identified by filename.

## Deployment

```sh
fastgpu deploy /workspace/models/my-model.gguf --name my-model
fastgpu deploy /workspace/models/my-model.gguf --engine ollama --name my-model
fastgpu deploy /home/gpu/checkpoints/experiment --engine vllm --name my-model
fastgpu deploy qwen3.8-27b
fastgpu deploy nanbeige4.3-33b
```

| Option | Purpose |
| --- | --- |
| `--engine` | `llama.cpp` (default), `ollama`, or `vllm` |
| `--name` | API model name; otherwise derived from the path |
| `--context` | Context length, 512–262,144 tokens; requires enough VRAM |
| `--local-port` | Local tunnel port, default 8080 |
| `--max-price` | Compute cap for a newly created Pod |
| `--min-vram` | Minimum VRAM when creating a Pod |
| `--hf` | Interpret the target as a Hugging Face model ID for vLLM |
| `--tokenizer` | Hugging Face tokenizer ID or persistent path for vLLM |
| `--arg` | Additional llama.cpp/vLLM argument; repeatable |
| `--modelfile` | Remote Modelfile on the volume for Ollama |

`deploy` starts or reuses a Pod, prepares the requested model, stops any previous service, waits for the new service, and opens the tunnel. All engines use the remote loopback interface on port 8080, with one managed service at a time. Training and conversion are not automatic.

See [models](CLI.md) for format selection and complete examples.

```sh
fastgpu model pull qwen3.8-27b
fastgpu model pull nanbeige4.3-33b
fastgpu model list
fastgpu model list /workspace/models
fastgpu logs
fastgpu logs --follow
fastgpu logs --pod
fastgpu tunnel --close
fastgpu tunnel --local-port 8081
fastgpu undeploy
```

`model pull [MODEL]` downloads and verifies a preset on the selected Pod; omitting the model selects Qwen. Supported presets are `qwen3.8-27b` and `nanbeige4.3-33b`.

`model list [DIRECTORY]` recursively lists GGUF files in the managed directory by default, or in the supplied directory on the volume (`home:` and `vol:` are supported). The `DEPLOY TARGET` column contains the preset alias or quoted absolute path to pass to `fastgpu deploy`. Partial downloads, verification metadata, and symlinks are excluded. Listing requires a reachable Pod.

`logs --pod` reads provider events, which is useful before SSH is ready; it stays open until Ctrl-C. Other logs belong to the inference service. Tunnels use background OpenSSH processes: closing a tunnel does not stop the service or Pod. `undeploy` stops the service and closes its tunnel, but the GPU remains billable.

## Volumes

`fastgpu volumes` and `fastgpu volume` are equivalent.

| Subcommand | Effect |
| --- | --- |
| `list` | List all pages of Network Volumes |
| `show ID_OR_NAME` | Show ID, name, size, data center, and type |
| `use ID_OR_NAME` | Save the selection; reject a change incompatible with the active Pod |
| `create NAME --size GB --datacenter DC [--type STANDARD]` | Create 10–4,096 GB of storage |
| `rename ID_OR_NAME NEW_NAME` | Rename a volume |
| `resize ID_OR_NAME GB` | Increase capacity; shrinking is not supported |
| `delete ID_OR_NAME --confirm ID` | Permanently delete unattached, unprotected storage |

Duplicate names require an ID. Choose STANDARD/HIGH_PERFORMANCE when creating a volume. Storage changes may increase costs. There is no special personal volume name: protection covers the configured ID, the managed Pod's volume, and volumes attached to account Pods.

## Diagnostics and recovery

`fastgpu doctor` checks SSH, rsync, and API access; VS Code is optional. `fastgpu doctor --remote` checks installed programs, imports vLLM/PyTorch, and performs a CUDA calculation. It does not authenticate accounts or automatically run generation tests for every engine.

`up` waits up to 25 minutes for the API and SSH. On failure, state remains available to `status`, `logs --pod`, and `down`. Service readiness has a 20-minute wait; Ollama preparation/import also has a timeout. The preset download can take longer.

If a creation POST has an uncertain outcome, the CLI does not repeat it blindly: it reconciles the operation name first. Use `fastgpu recover` to inspect state and `fastgpu pods` to check resources. `recover --clear` removes an uncertain reference only after the API check and the user's manual verification.

A client error, closed terminal, or model failure does not guarantee Pod termination. Check its status and use `down` when finished.


## Modelli e inferenza


## Workflow

1. Run training with your own tools over SSH or in VS Code.
2. Save a GGUF file or a complete checkpoint in the persistent home directory or under `/workspace`.
3. Choose an engine and run `fastgpu deploy` from your local computer.
4. Call the model at `http://127.0.0.1:8080/v1`.

Deployment paths refer to the **remote machine**. If a GGUF is on your local computer, copy it first with `fastgpu push ./my-model.gguf vol:models`.

## Formats and engines

| Engine | Main input | Customization |
| --- | --- | --- |
| llama.cpp | GGUF file | Repeat `--arg` for executable options |
| Ollama | GGUF or a supported Safetensors directory | `--modelfile` pointing to a file on the volume |
| vLLM | Hugging Face checkpoint directory or remote model ID | `--arg`, `--tokenizer`, `--hf` |
| vLLM with GGUF | Supported GGUF and an explicit compatible tokenizer | Experimental, model-dependent support |

Engines differ in supported architectures, quantization formats, and templates. The same file is not automatically compatible with all three. The CLI lets you supply the required settings; training and format conversion remain manual.

## llama.cpp

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine llama.cpp --name my-model --context 4096
```

You can also use `home:checkpoints/my-model.gguf`, `vol:models/my-model.gguf`, or `/home/gpu/checkpoints/my-model.gguf`. Unprefixed relative paths are relative to the remote home directory. The resolved path must remain on the Network Volume, including after following symlinks.

By default, the server uses one slot, offloads all layers to the GPU, disables automatic context fitting, and uses the model's Jinja template. To add model-specific options:

```sh
fastgpu deploy /workspace/models/my-model.gguf --name my-model \
  --context 4096 --arg=--threads --arg=8
```

Each `--arg` supplies one argument directly to the process without shell interpretation. Use the `=` form for values starting with `--`. Address, port, primary model path, alias, authentication, and configuration options managed by the CLI cannot be overridden through `--arg`.

`--name` sets the model name in the API. If omitted, the name is derived from the filename, normalized, and stripped of `.gguf`.

## Ollama

Ollama is included in the image. Its server starts when you select this engine:

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine ollama --name my-model --context 4096
```

The CLI starts `ollama serve` on the remote loopback interface, imports the model, and checks that it loads. It generates a Modelfile with `FROM` pointing to the target path and `PARAMETER num_ctx` set to the chosen context.

For custom templates, parameters, or adapters, save a Modelfile on the volume:

```text
FROM /workspace/models/my-model.gguf
PARAMETER num_ctx 4096
PARAMETER temperature 0.6
SYSTEM Answer in English.
```

Then run:

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine ollama --name my-model \
  --modelfile /workspace/models/Modelfile
```

With `--modelfile`, that file controls the content and parameters: set `num_ctx` there too. Keep every artifact referenced by the Modelfile on the volume. Import may create an initial copy in Ollama's storage format; it stays under `/workspace/.gpu/ollama/models`.

Both the OpenAI-compatible API and native endpoints such as `http://127.0.0.1:8080/api/tags` are available through the same tunnel. Ollama may show the model with a `:latest` suffix in its catalog.

## vLLM with checkpoints

An exported checkpoint must contain the required configuration, weights, and tokenizer. For LoRA/QLoRA, prepare a complete checkpoint or supply the vLLM options needed for your setup.

```sh
fastgpu deploy /home/gpu/checkpoints/experiment --engine vllm --name my-model \
  --context 4096 --arg=--enforce-eager \
  --arg=--gpu-memory-utilization --arg=0.85
```

For a model from Hugging Face:

```sh
fastgpu deploy ORGANIZATION/MODEL --engine vllm --hf --name my-model \
  --context 4096 --arg=--revision --arg=REVISION
```

`--hf` distinguishes a remote model ID from a path. Downloads are cached in the persistent home directory. Private and gated models require Hugging Face authentication on the Pod; the CLI does not automatically copy tokens from your local computer.

The image defaults to the native sampler so sampling does not depend on compiling FlashInfer with `nvcc`. Other kernels or models may require additional image dependencies.

## vLLM with GGUF

The [vLLM documentation](https://docs.vllm.ai/en/latest/features/quantization/gguf/) describes GGUF support as experimental. Specify the original model's tokenizer or a compatible persistent tokenizer path:

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine vllm --name my-model \
  --tokenizer ORGANIZATION/BASE_MODEL --context 4096 \
  --arg=--enforce-eager
```

If the model also requires a separate HF configuration, supply it with `--arg=--hf-config-path --arg=/workspace/models/configuration`. Check that your vLLM version and GPU support the architecture and quantization. If they do not, use the HF checkpoint or a supported GGUF with llama.cpp/Ollama.

## Included presets

```sh
fastgpu deploy qwen3.8-27b
fastgpu deploy nanbeige4.3-33b
```

| Deploy alias | GGUF | Download size |
| --- | --- | --- |
| `qwen3.8-27b` | Unsloth `Qwen3.8-27B-UD-Q4_K_M.gguf` | 16.46 GB |
| `nanbeige4.3-33b` | Bartowski `Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf` | 20.79 GB |

Each preset downloads to `/workspace/.gpu/models` if absent, verifies its pinned size and SHA-256, and uses llama.cpp by default. Later deployments reuse the file. `fastgpu up` and `fastgpu deploy` without a model still select Qwen. Other engines have their own format, template, and tokenizer requirements.

The default context is 131,072 tokens unless overridden by the saved profile or `--context`. Nanbeige's [configuration](https://huggingface.co/Nanbeige/Nanbeige4.3-33B/blob/main/config.json) implies about 26 GiB of FP16 KV cache at that context, in addition to the weights and runtime buffers. A 24 GB GPU is insufficient for full GPU offload at 128k with that cache; choose a larger GPU or suitable cache settings. `--min-vram` only affects creation of a new Pod. Nanbeige generation and tool calling still require validation on the selected GPU and image.

Both the local CLI and the worker in the Docker image must contain the new preset and listing commands. Rebuild the client and image when upgrading from a build that only supports Qwen; see [image builds](CLI.md).

## List installed GGUFs

```sh
fastgpu model list
fastgpu model list /workspace/models
fastgpu model list home:checkpoints
```

With no directory, the command searches `/workspace/.gpu/models`. An explicit directory selects another location on the volume. Subdirectories are included; symlinks, partial downloads, and verification metadata are excluded. A reachable Pod is required.

The `DEPLOY TARGET` column shows the short alias for presets in the managed directory and a shell-quoted absolute path for other GGUFs. Copy that value after `fastgpu deploy`. The listing covers GGUF files; checkpoint directories and Ollama's imported blob store are separate storage formats.

## Endpoints and engine changes

```sh
fastgpu deploy /workspace/models/my-model.gguf --name my-model --local-port 8081
curl http://127.0.0.1:8081/v1/models
```

Inference listens on the remote loopback interface. RunPod exposes SSH, and the tunnel makes the API available only on your local loopback interface. `--local-port` changes the local port; the managed remote port remains 8080.

The CLI manages one inference service per Pod. A different deployment stops the previous service, waits for the new engine, and opens the tunnel. Switching can interrupt requests in progress. An identical deployment that is already ready reuses the service.

Do not modify weights while they are being served. Finish exporting to a new file, then deploy that path. GGUF verification caches SHA-256 using file size and modification time; directories are identified using file metadata. The volume is assumed to be modified by trusted processes.

```sh
fastgpu logs
fastgpu logs --follow
fastgpu undeploy
```

`undeploy` stops the service and closes the tunnel. Use `fastgpu down` to terminate the GPU Pod as well. Model or memory errors leave the Pod available for inspection; they do not automatically end billing.

## Sources

- [llama.cpp server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server)
- [Importing models in Ollama](https://docs.ollama.com/import)
- [Ollama Modelfile format](https://docs.ollama.com/modelfile)
- [Ollama API compatibility](https://docs.ollama.com/api/openai-compatibility)
- [GGUF models in vLLM](https://docs.vllm.ai/en/latest/features/quantization/gguf/)


## Persistenza e lavoro remoto


## Volume mount

The selected Network Volume is attached at `/workspace` when the Pod is created. The image stores its environment under `/workspace/.gpu` and creates this symlink:

```text
/home/gpu -> /workspace/.gpu/home
```

Writing to the remote home directory therefore writes to the volume. The volume remains after `fastgpu down`; a new Pod mounting the same volume will find those files again.

| Path | Contents |
| --- | --- |
| `/home/gpu/progetti` | Default project directory created by the current image |
| `/home/gpu/datasets` | Training data |
| `/home/gpu/checkpoints` | Checkpoints, exports, and outputs |
| `/home/gpu/.cache/huggingface` | Hugging Face weights, tokenizers, and cache |
| `/home/gpu/.cache/torch` | PyTorch cache |
| `/home/gpu/.cache/uv` | uv cache |
| `/workspace/.gpu/models` | Included model, partial downloads, and verification metadata |
| `/workspace/.gpu/ollama/models` | Persistent Ollama model store |
| `/workspace/.gpu/logs` | Rotating inference logs |
| `/workspace/.gpu/deployment.json` | Last deployment configuration |

The examples below create a `projects` directory; any directory under the remote home is persistent. You can also use `/workspace/models`, `/workspace/datasets`, or other directories on the volume. `/tmp`, `/opt`, `/usr`, and other container-disk locations are outside the Network Volume. Save outputs you want to keep under the home directory or `/workspace`.

## Clone a repository once

```sh
fastgpu up --skip-model
fastgpu mkdir projects
fastgpu shell --cwd home:projects git clone REPOSITORY_URL experiment
fastgpu code projects/experiment
```

The repository is cloned on the Pod and stays on the volume. After recreating the Pod, open it directly with `fastgpu code projects/experiment`. Run `git pull` when you want to update it; `up` does not clone or synchronize code automatically.

For private repositories, authenticate Git or `gh` on the Pod with your account. Credentials saved in the home directory are persistent: anyone able to mount the same volume can access those files.

## Copy a local project

```sh
fastgpu push ./experiment/ home:projects/experiment
fastgpu pull home:projects/experiment/ ./local-copy
```

Transfers are explicit. `push` uses rsync and reuses existing files when possible; `up` and `deploy` do not copy the project again. The destination is a directory. A trailing `/` on the source copies its contents; without it, rsync also copies the directory itself.

Transfers do not use `--delete`: files found only at the destination are kept. `.venv`, `__pycache__`, `.env`, `.env.*`, `*.pem`, and `*.key` are excluded. Create Python environments on the Pod rather than copying a virtual environment built on your local computer.

## Run training

```sh
fastgpu shell
fastgpu shell --cwd home:projects/experiment python train.py
fastgpu code projects/experiment
```

Without arguments, `fastgpu shell` opens an interactive SSH terminal in the home directory. Enter your project and work normally. In VS Code, use the remote terminal in the opened folder.

Your script must save checkpoints and GGUF files in the home directory or on the volume. If `train.py` writes relative paths, `--cwd home:projects/experiment` keeps them inside the persistent project. If the script accepts an output directory, point it to a location such as `/home/gpu/checkpoints/experiment`.

Training, merging LoRA adapters, and GGUF conversion remain your responsibility. For LoRA/QLoRA, prepare the complete model or the GGUF required by your chosen engine.

To keep training running after SSH disconnects:

```sh
fastgpu shell --tty tmux new-session -A -s training
```

Start the script inside tmux and detach using tmux's key sequence. The Pod remains billable while it is running.

## Models and caches

The Qwen and Nanbeige presets have pinned URLs, revisions, and SHA-256 checksums. Downloads use a resumable `.partial` file and are renamed after verification. Later deployments reuse the existing GGUF.

For your own GGUF, `fastgpu deploy /workspace/models/my-model.gguf` reads that file directly and caches its verification result. It does not download or copy it. Finish saving before deployment; if the file changes, run a new deploy to apply the new version.

vLLM reads local checkpoints or uses the Hugging Face cache in the home directory. An unpinned Hugging Face model ID may resolve to updated weights when its remote revision changes. To reproduce the weights, pass `--arg=--revision --arg=REVISION` or use a saved local checkpoint.

Ollama imports a GGUF or checkpoint into its own blob store. The first import may require additional storage. Blobs and manifests remain on the volume and are reused instead of being recreated on ephemeral storage for every Pod.

## Check data locations

```sh
fastgpu info
fastgpu info --live
fastgpu shell pwd
fastgpu shell readlink -f /home/gpu
fastgpu shell ls -lh /workspace/.gpu/models
```

`info --live` shows the resolved home directory. Network Volume capacity comes from the API; on some shared filesystems, `df` reports the storage pool rather than the individual volume's quota.

## Concurrency and retention

One active `fastgpu` environment per volume is supported. A lock protects the home directory; other programs that ignore these locks can still modify the same files.

The image does not recursively `chown` the mount. Some Network Volumes remap UID/GID and reject POSIX ownership changes; startup checks that managed directories are writable. Do not rely on volume `chmod` settings to isolate people who can access the same storage.

`down` preserves the volume and its data. `volumes delete` is an explicit, irreversible deletion of unprotected storage. Retained storage remains billable.


## Immagine Docker


The image contains software and libraries. Repositories, checkpoints, models, and caches live on the Network Volume. The build copies only the required source and installation scripts; keys, local profiles, and logs are excluded from its context.

## Components

- Ubuntu 24.04 with CUDA runtime 13.0.2, requiring a compatible NVIDIA host.
- Python 3.12, uv 0.13.6, and a vLLM 0.33.0 environment at `/opt/vllm`.
- llama.cpp release `b12034`, Linux x64 CUDA 12.8 binaries, and matching runtime libraries at `/opt/llama`.
- Ollama 0.40.13 for Linux x86-64, including libraries from its official distribution.
- Git, GitHub CLI 2.103.3, Codex CLI 0.166.0, OpenSSH server, rsync, tmux, and build tools.
- Python headers and Ninja for compiling Triton components.

The Rust `fastgpu` CLI is compiled from `src/` in the `rust:1.91.1-bookworm` build stage using `cargo build --release --locked`, then installed at `/usr/local/bin/fastgpu`. The build includes `Cargo.toml` and `Cargo.lock`; the final CUDA image does not include Cargo or the Rust compiler. The binary uses the system C library and rustls for TLS. Supervisor invokes `fastgpu worker serve`. Main downloaded archives have pinned versions and SHA-256 checksums. Direct Python dependencies are pinned; transitive and apt dependencies can change between builds. `/opt/python-installed.txt` records the resolved Python versions.

vLLM uses its native sampler through `VLLM_USE_FLASHINFER_SAMPLER=0`; inference still runs on the GPU. This avoids requiring `nvcc` for the FlashInfer sampler, as described in the [NVIDIA documentation](https://docs.dynamo.nvidia.com/dynamo/v1.3.0/getting-started/local-installation). Add compatible toolkit/kernel-cache dependencies and repeat model tests if you need specific FlashInfer kernels.

## Where to add software

| File | What to change |
| --- | --- |
| `docker/packages.apt` | Ubuntu packages, one per line |
| `docker/requirements.txt` | Direct dependencies of the inference Python environment |
| `docker/install.d/*.sh` | Custom installation scripts, executed alphabetically |
| `docker/Dockerfile` | Layer composition, base image, and environment variables |
| `docker/entrypoint.sh` | Volume, home, cache, and SSH initialization |
| `docker/supervisord.conf` | Services and log rotation |

Add Ubuntu packages to `packages.apt`. For software with its own distribution, add a numbered script under `install.d`; Ollama, for example, is installed by `40-ollama.sh`.

Scripts must be non-interactive, stop on errors, and install into `/usr`, `/usr/local`, or `/opt`. Pin versions and verify checksums when available. Keep tokens out of scripts, build arguments, and the Dockerfile.

Create uv environments in the persistent home directory for project-specific dependencies. Build a new image for system-level additions. Installing packages on a running container's disk does not make those changes persistent across new Pods.

## Build locally

```sh
make image IMAGE=YOUR_ACCOUNT/fastgpu-workspace:0.3.0
```

To check just the Rust CLI build stage:

```sh
docker build --target cli -f docker/Dockerfile -t fastgpu-cli-test .
docker run --rm fastgpu-cli-test /fastgpu --version
```

The image is large because it includes three inference engines and GPU libraries. Allow enough disk space for layers, extraction, and Docker's cache. Docker is not required to use the client alone.

## Registry and image publishing

RunPod must be able to pull the image from a registry. To publish your own:

```sh
docker login
docker push YOUR_ACCOUNT/fastgpu-workspace:0.3.0
docker inspect --format '{{index .RepoDigests 0}}' YOUR_ACCOUNT/fastgpu-workspace:0.3.0
fastgpu init --image YOUR_ACCOUNT/fastgpu-workspace@sha256:DIGEST
```

Replace these placeholders with real references. A digest selects an exact image; use a new tag for each release. For private images, configure a registry credential in RunPod and pass its ID to `fastgpu init --registry`.

Publishing an image does not publish the source on GitHub. Version 0.3.0 must contain the Rust `fastgpu` worker matching the local client. The default reference is `felipeoperti/fastgpu-workspace:0.3.0`, but this source migration does not publish that tag or assign it a digest. The older 0.2.1 image remains a historical validation baseline, not a validated Rust image; see [validation](CLI.md).

## Startup and persistence

The container requires `/workspace` to be mounted and `PUBLIC_KEY` to be set; `fastgpu up` supplies both. The remote user is `gpu`, UID/GID 2000. `/home/gpu` points to `/workspace/.gpu/home`; startup prepares project, dataset, checkpoint, and cache directories.

`HF_HOME`, `TORCH_HOME`, and `UV_CACHE_DIR` point to caches in the persistent home directory. `OLLAMA_MODELS` is `/workspace/.gpu/ollama/models`; `OLLAMA_HOST` selects the loopback interface on managed port 8080. The same settings are available in SSH shells.

The `.gpu` namespace must be absent/empty on first use or already initialized with the expected format. Startup does not recursively change ownership on the volume. If the filesystem rejects `chown`, startup checks that the remote user can write to the managed directories.

Operational authorized keys are stored on the container's local disk. SSH host keys have a persistent copy per Pod ID and are prepared locally with SSH's required permissions at startup. Network Volume Unix permissions do not isolate people who can access the same storage.

Supervisor starts SSH. The `inference` service starts only through `fastgpu deploy`, selecting its engine from the persistent manifest. After Pod recreation, data remains and inference is reactivated by deploying. Logs rotate at 20 MB with three backups.

## Validate an extension

1. Build the image with a new tag.
2. Verify that startup without a volume is rejected and that the new programs launch.
3. Push the image to the registry and record its digest.
4. Create a test Pod with a Network Volume and the matching CLI version.
5. Run `fastgpu doctor --remote`, `fastgpu info --live`, shell commands, transfers, and a VS Code connection.
6. Run a real generation with the engines and models affected by the change.
7. Terminate the test Pod with `fastgpu down` and verify that the volume remains.

Testing one GPU or model does not certify every architecture, quantization format, or future option.


## Pacchetto npm locale


The npm package contains the compiled Rust executable and a small Node.js launcher. Its current name is `fastgpu`, version 0.3.0. It is marked `private: true`, so npm refuses registry publication. No npm account or registry login is required for local packaging or installation.

## Build and install locally

From the source checkout:

```sh
make npm-package
npm install -g ./dist/fastgpu-0.3.0.tgz
fastgpu --version
```

`make npm-package` runs `npm pack`; its `prepack` hook builds the release binary with `make build`, verifies the package/Cargo/binary versions and Linux x64 ELF header, and copies the executable into `npm/bin/fastgpu`. You can also run `npm pack` directly; it writes the `.tgz` into the current directory.

Creating a package requires Rust, Cargo, make, a C toolchain, Node.js, and npm. Installing an existing `.tgz` requires only Node.js and npm. The package has no npm dependencies and no install-time scripts. OpenSSH and rsync are still needed for remote operations.

The supported Node.js versions are 22.15+ within the 22.x series, and 24 or later. The launcher uses `process.execve` to replace itself with the Rust executable, retaining its standard streams and process identity. Interactive terminals, signals, and exit status are handled by the Rust CLI.

## Linux compatibility

This package supports Linux x64 with glibc. A binary built on your own computer inherits that build environment's glibc requirements. The local Ubuntu build currently requires glibc 2.39; do not assume it works on older distributions.

To package the Debian-based Docker CLI build, use:

```sh
docker build --target cli -f docker/Dockerfile -t fastgpu-cli-npm .
mkdir -p .local/npm-build
npm_container=$(docker create fastgpu-cli-npm)
docker cp "$npm_container":/fastgpu .local/npm-build/fastgpu
docker rm "$npm_container"
FASTGPU_NPM_BINARY="$PWD/.local/npm-build/fastgpu" make npm-package
```

`FASTGPU_NPM_BINARY` selects an already compiled executable; version and architecture checks still apply. This build uses Debian Bookworm and requires glibc 2.34 or newer according to its imported symbols. The archive is intended for glibc-based Linux systems, including Debian 12; Alpine/musl, ARM64, macOS, and Windows are not supported by this package.

The Docker CLI build does not build or publish the CUDA worker image. That image remains a separate artifact and must match the client version for deployment.

## Verify the package

```sh
npm test
```

Tests cover argument quoting, environment and working-directory preservation, standard streams, exit codes, SIGINT/SIGTERM, unsupported runtimes, and missing executables. Packaging tests build an archive, inspect its complete file list and executable permissions, and install it offline into a temporary npm prefix. They also reject incompatible or incorrectly versioned input binaries.

Use `FASTGPU_NPM_BINARY=/absolute/path/to/fastgpu npm test` to test a prebuilt executable instead of rebuilding locally. Test installations use temporary directories and do not modify your existing CLI installation or RunPod resources.

Only the launcher, executable, package metadata, README, license, and documentation are included. Generated binaries, tarballs, and `node_modules` are ignored by Git. Future registry distribution requires choosing the final package name/scope and explicitly removing `private: true`.


## Architettura


## Components

The `fastgpu` Rust client uses RunPod REST API v2, OpenSSH, and rsync. Its entry point is `src/main.rs`, with shared client and worker modules exposed through `src/lib.rs`. The same executable contains internal `worker` commands used inside the container for downloads, installation checks, and service startup. No additional control server is required.

Commands and argument parsing are in `src/cli.rs` (Clap); APIs and resource selection in `src/api.rs` and `src/lifecycle.rs`; SSH, VS Code, and transfers in `src/ssh.rs`; child processes and terminal restoration in `src/process.rs`; connection information in `src/info.rs`; inference engines in `src/engines.rs`; presets and listing in `src/models.rs`; and downloads and remote checks in `src/worker.rs`. `src/config.rs` preserves the existing JSON profile format.

Tokio runs HTTP requests, subprocesses, timeouts, and signal cancellation. Reqwest uses rustls for HTTPS. Dropping a cancelled subprocess kills its child, and a terminal guard restores the original terminal state. Filesystem locks are released when their file handles close. Cargo builds the same executable for the local client and container; `Cargo.lock` pins dependencies.

## Storage

The Network Volume is mounted at `/workspace` and survives Pod termination. The home directory is a symlink to `/workspace/.gpu/home`. This namespace also holds models, the Ollama store, logs, and the deployment manifest. The image supplies software while the volume retains user data.

The volume's data center constrains Pod placement. A container disk or Pod volume disk does not replace this persistence. One managed environment per volume is supported; a lock prevents concurrent startup against the same home directory.

See [persistence](CLI.md) for the layout and workflows.

## Connections and information

The public port is `22/tcp`. The image's SSH user is `gpu`; SSH uses dedicated keys and does not forward the local agent. The client generates a separate configuration file and a Pod-ID-based alias, refreshing its current port mapping.

`info` shows operational provider details and omits credential-bearing containers. `connect` generates direct SSH and TCP forwarding commands. Engines listen on the remote loopback interface and are exposed locally through a tunnel, without publishing their inference port on RunPod.

## Deployment

The client sends the worker a JSON manifest containing the engine, target, alias, context length, and options. The worker validates persistent paths, computes or reuses the model fingerprint, and serializes changes with a lock. One supervisor service, `inference`, runs the selected engine.

llama.cpp reads GGUF directly. vLLM reads a checkpoint or Hugging Face model with a cache in the home directory; GGUF requires a tokenizer. Ollama starts its server and imports the model into its persistent store. A custom Modelfile can define model settings.

Readiness checks include the served alias. For Ollama, the model must be loaded as well as present in its catalog. The tunnel opens after readiness is confirmed.

An identical manifest with an already-ready service is reused. Changing the target, model, or engine stops the previous service before starting the next. The CLI does not perform training or conversion.

## Model verification

The Qwen preset uses:

- Repository: `unsloth/Qwen3.8-27B-GGUF`.
- Revision: `4ca720788d1e01f1bff70c033e0d0028fd02e502`.
- File: `Qwen3.8-27B-UD-Q4_K_M.gguf`, 16,464,440,224 bytes.
- SHA-256: `322e194ff79741c7baa497c240f677f54b201b0efab44ca8e50f122b39123482`.

The Nanbeige preset (`nanbeige4.3-33b`) uses:

- Repository: `bartowski/Nanbeige_Nanbeige4.3-33B-GGUF`.
- Revision: `9beb68421fdf7be095093a6888325fa2c83ddd38`.
- File: `Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf`, 20,788,825,216 bytes.
- SHA-256: `2cef79855ca47ee6571e166db32a07089e34f3cf78e0397b262d124ddb814c37`.

Partial downloads resume through HTTP Range and are renamed after verification. Custom GGUF verification is cached using size and modification time. Checkpoints are identified by file metadata; Hugging Face IDs require an explicit revision to pin the weights.

These caches assume trusted processes on the volume. They do not defend against deliberate changes to files and metadata. Do not modify weights while they are being served.

## Errors and recovery

Local configuration and state use temporary files and rename; SSH configuration is replaced atomically too. A lock serializes profile mutations. API reads complete pagination and retry some transient errors; writes with uncertain outcomes are not repeated automatically.

Creation records an operation name first. If the response is lost, the client looks for that resource before creating another. The Pod ID is saved before waiting for SSH. Startup failures leave the Pod available for inspection and termination.

`down` deletes only the managed Pod, verifies the Network Volume's existence, then clears local state. Volume deletion is a separate operation requiring ID confirmation and checks for protected/attached volumes.

## Current limitations

- Linux client, one NVIDIA GPU per Pod, Secure Cloud On-Demand.
- One managed Pod and inference service at a time; creation checks the account for existing `gpu-*` Pods. The profile lock serializes local operations, but account checks cannot make simultaneous creation from different profiles or computers atomic.
- Model compatibility depends on engine, format, GPU, and options.
- Training, GGUF export, and LoRA management remain the user's responsibility.
- No continuous synchronization, S3 backend, or autonomous shutdown timer.
- No capacity reservation after stopping a Pod.
- Personal GitHub, Codex, and Hugging Face login is separate from installing their tools.

## Sources

- [RunPod API v2](https://docs.runpod.io/api-reference-v2/overview)
- [Pod details](https://docs.runpod.io/api-reference-v2/pods/get-a-pod)
- [Network Volumes](https://docs.runpod.io/storage/network-volumes)
- [RunPod SSH](https://docs.runpod.io/pods/configuration/use-ssh)
- [llama.cpp](https://github.com/ggml-org/llama.cpp)
- [Ollama](https://docs.ollama.com/import)
- [vLLM](https://docs.vllm.ai/en/latest/)


## Verifiche e limiti


This document distinguishes completed tests from functionality that still needs validation. Local operational reports may contain account identifiers and are excluded from repository documentation. The historical sections below describe the previous implementation; their GPU and image results do not validate the Rust migration.

## Local npm package — November 15, 2026

Prepared `fastgpu-0.3.0.tgz` for Linux x64 with glibc. The package is private and has not been published to a registry.

- Nine npm tests passed: launcher arguments, streams, environment, working directory, process identity, exit status, signals, runtime errors, binary validation, and offline package installation.
- The archive file list contains only the executable, launcher, package metadata, README, license, and documentation; executable permissions are retained.
- `make npm-package` builds and packages the native release. `FASTGPU_NPM_BINARY` was also tested with the executable from the Docker CLI stage.
- The prepared archive uses the Docker-built binary, whose imported glibc symbols require at least 2.34. Offline installation and `--version`, `--help`, and `config` passed both locally and in a network-disabled Debian Bookworm container, with Cargo absent from the installation PATH.
- Test installations used temporary prefixes. Existing local installations and RunPod resources were not modified.

See [npm packaging](CLI.md) for installation and rebuilding instructions. These checks do not validate GPU inference.

## Rust migration — November 15, 2026

Version 0.3.0 replaces the local CLI and container worker with Rust. Go sources and module files have been removed; Cargo, the Makefile, and the Docker CLI stage build the Rust executable.

Completed local checks:

- `make test`: 43 tests passed (40 library tests, including the subprocess fixture, and 3 executable tests), with `cargo fmt` and Clippy passing without warnings.
- `make build`: release binary built as `bin/fastgpu`.
- `make install DESTDIR=...`: installation verified in a temporary staging directory; the staged binary reports `fastgpu version 0.3.0`.
- Command help, aliases, argument validation, JSON profile compatibility, file permissions, and profile locks.
- Mock RunPod API tests for pagination, read retries, creation reconciliation, GPU sizes, price caps, existing-Pod reuse, and protected-volume deletion.
- Child-process tests for SSH command arguments, exit-status propagation, Pod resume, VS Code paths, rsync exclusions, and cancellation.
- Model downloads against local HTTP fixtures, including Range handling, checksum rejection, and verified-file reuse; model listing, symlink containment, fingerprints, and engine readiness checks.
- Docker `cli` stage built successfully with Rust 1.91.1; `/fastgpu --version` reports 0.3.0 inside the container.
- Worker smoke checks in a network-disabled container with temporary storage and stub supervisor/engine commands: GGUF deployment, checkpoint deployment, engine arguments, loopback settings, model listing, and rejection of changed weights. These checks do not run real inference.
- Local documentation links and `git diff --check`.

No RunPod resources were created or modified during the migration. The complete CUDA image and GPU inference have not been validated for Rust 0.3.0. The new image tag has not been published; build and publish a matching image before deploying with the Rust client. The historical image digests below refer to earlier releases.

## Historical baseline before the Rust migration

Version 0.1.3 was tested locally and on RunPod using a 32 GB RTX 5090 and a 96 GB RTX PRO 6000:

- Pod creation and termination with a Network Volume.
- Volume and file retention after Pod recreation.
- Volume management through real APIs using temporary storage.
- SSH, transfers verified by SHA-256, arguments containing spaces, and exit-code propagation.
- VS Code Remote SSH connections.
- Python/uv, GitHub CLI, Codex CLI, vLLM/PyTorch, and CUDA computation.
- Resumed and verified Qwen preset downloads and llama.cpp inference from localhost.
- vLLM generation with a small model and the native sampler.
- Tunnel closure and termination of only the Pods created for testing.

Version 0.2.1 extends deployment to custom paths and three engines. Its additional tests are recorded below.

## Version 0.2.1 — November 2, 2026

This record predates the CLI rename. Commands below use the current `fastgpu` spelling; the live tests ran with the previous executable. The image and digest recorded here contain `gpu`, not `fastgpu`, and do not validate the rebuilt image required by the renamed client.

Published image: `felipeoperti/gpu-workspace:0.2.1`.

Validated digest: `sha256:3460ff504e8fa46a87d543f1cd8a442db9933599f5dcba10da1eb28070db9bc1`.

Complete engine testing ran on a 96 GB RTX PRO 6000 Blackwell Workstation using On-Demand compute. Machine information and shell access were also tested on a 32 GB RTX PRO 4500. Local Docker tests covered Ollama, llama.cpp, and reuse of the Ollama store after container restart.

| Area | Observed result |
| --- | --- |
| Go build and tests | Passed, including race detection and `go vet` |
| Help | General and subcommand help available |
| `fastgpu info --live` | API/GPU details and home resolved under `/workspace/.gpu/home`; credential-bearing containers excluded |
| `fastgpu connect` | Generated SSH command executed successfully |
| Generated TCP forwarding | Connection through the tunnel received an SSH banner |
| `fastgpu shell` | Interactive terminal, command execution, and exit code 0 |
| Remote Python script | Ran from the home directory; wrote a checkpoint-directory result retrieved identically through `pull` |
| Custom GGUF | Copied once into the persistent home directory and verified by SHA-256 |
| llama.cpp | Deployment by GGUF path and generation from localhost succeeded |
| Identical deployment | Service process remained unchanged |
| Ollama | Same GGUF imported; localhost generation and GPU use confirmed by `/api/ps` |
| Custom Modelfile | Import and generation verified in Docker with configuration stored on the volume |
| Ollama cache | After Docker restart, the model was found without another import |
| vLLM | Local Hugging Face checkpoint loaded and localhost generation succeeded |
| Engine switching | llama.cpp → Ollama → vLLM → llama.cpp succeeded |
| Qwen3.8-27B preset | GGUF reused from the volume; localhost request for 2+2 returned `4` |
| Cleanup | Service stopped, tunnel closed, test Pod terminated, and volume verified present |

SmolLM2-135M-Instruct was used for the three-engine tests: Q4_K_M GGUF for llama.cpp/Ollama and a complete checkpoint for vLLM. Requests returned nonempty text. vLLM used a 512-token context, eager mode, and GPU memory utilization of 0.2. Qwen used the default context with thinking disabled in the request.

Testing led to a terminal-handling correction: OpenSSH sets terminal modes while the CLI keeps the original state for restoration. Recognition of explicit RunPod capacity rejection was also tested; uncertain creation outcomes do not authorize automatic write retries.

No test Pods remained active at completion, and the profile had no selected Pod. Models, caches, and small test files remained on the volume. Detailed logs are under the local `.local/v020-e2e/` directory, which is excluded from the repository. Personal storage names, addresses, and identifiers are not published in the documentation.

## CLI rename — November 2, 2026

Local validation of the `fastgpu` rename passed:

- `make test`: race-detector tests and `go vet`.
- `make build` and `make install` into a temporary staging directory.
- `fastgpu --version`: `fastgpu version 0.2.1` from the built and staged binaries.
- All 44 help pages, including nested commands, the `volume` alias, and the hidden worker; default image and error prefix checks.
- Source and documentation audit for old CLI invocations, plus local documentation links.
- Docker's `cli` build stage and `/fastgpu --version` inside that image.

At this stage, only Docker's CLI build stage had been validated. The subsequent complete image rebuild and publication are recorded below. RunPod inference tests have not been repeated after the rename.

## English CLI output — November 2, 2026

Help, argument placeholders, prompts, messages, errors, worker checks, and container startup messages are now in English. Informational JSON fields from `info` and `connect` are also English; see [the CLI reference](CLI.md).

`make test` passed, including race detection and `go vet`. `make install` updated the local executable. All 44 help pages and 10 local error paths were checked on the installed binary, and embedded Python and shell scripts passed syntax checks. Existing storage paths remain unchanged.

## Published renamed image — November 2, 2026

Published image: `felipeoperti/fastgpu-workspace:0.2.1`.

Digest: `sha256:a6b09b5bfe37a29a3d9885409d1dcef43511eceadd26f82df8b5ddcdb2ebbfdc`.

The complete image was rebuilt from `docker/Dockerfile`. Local tests verified startup with a temporary volume, the SSH daemon reaching RUNNING, and a real SSH login as `gpu` executing `fastgpu --version` successfully. Supervisor references `/usr/local/bin/fastgpu worker serve`; PyTorch and vLLM imports also passed.

Anonymous registry access was verified, and the published image matches the tested local image. The local profile was updated to the published digest, and a read-only RunPod startup plan selected the renamed image with a compatible GPU offer. No new RunPod was allocated for these checks; live GPU inference remains covered only by the earlier baseline.

## Repeat validation

```sh
make test
make build
./bin/fastgpu --help
./bin/fastgpu deploy --help
./bin/fastgpu volumes --help
```

`make test` runs `cargo fmt --all -- --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings`. Tests cover volume and offer selection, pagination, reconciliation after uncertain creation, deletion of only the Pod, quoting, locks, and SSE logs. New tests cover persistent paths, GGUF fingerprints, engine validation, readiness aliases, and information output without credential-bearing containers.

Preset tests cover both Qwen and Nanbeige through local HTTP downloads: resuming partial files, servers ignoring Range, checksum failures, and reuse of verified files. Listing tests cover preset aliases, custom paths with spaces and quotes, directory selection, and exclusion of partial files, metadata, and symlinks. Nanbeige has not yet been validated with real GPU inference, long context, thinking, or tool calls; the live results above describe the earlier Qwen validation.

For live testing, create a dedicated Pod with an explicitly selected volume. Check `doctor --remote`, `info --live`, `connect`, an interactive `shell`, a transfer round trip, VS Code, and generation with each affected engine. Run `undeploy` and `down` afterward, then verify that the volume exists and preexisting resources have not changed.

Tests do not automatically authenticate GitHub/Codex and do not certify every GPU, quantization format, model, or option combination. GGUF deployment with vLLM remains experimental and was not exercised in this validation; the checkpoint path was tested for vLLM. No automatic training was performed, as training management is outside the CLI's scope.

# fastgpu CLI reference

This manual describes the **0.3.0 Rust CLI** and the commands implemented in [src/cli.rs](../src/cli.rs). Commands run on your local computer unless marked as container internals. Names such as `POD_ID`, `ID_OR_NAME`, and `DATA_CENTER_ID` are placeholders to replace.

For a short introduction, use the [README](../README.md). For Rust lessons, implementation details, SSH internals, and public npm distribution, use [the development course](how-to.md).

## Contents

- [Installation](#installation)
- [Credentials and first setup](#credentials-and-first-setup)
- [Configuration and state](#configuration-and-state)
- [Command overview](#command-overview)
- [Help and configuration commands](#help-and-configuration-commands)
- [Pod lifecycle](#pod-lifecycle)
- [Connections and remote files](#connections-and-remote-files)
- [Models and inference](#models-and-inference)
- [Diagnostics](#diagnostics)
- [Network Volumes](#network-volumes)
- [Remote project workflow](#remote-project-workflow)
- [Container image](#container-image)
- [Internal worker commands](#internal-worker-commands)
- [Troubleshooting](#troubleshooting)
- [Validation and current limits](#validation-and-current-limits)

## Installation

### Native build

Use Linux, Rust 1.91 or newer with Cargo, a C toolchain, and make. Remote operations need OpenSSH and rsync. On Ubuntu/Debian, the relevant system packages are:

```sh
sudo apt-get update
sudo apt-get install -y openssh-client rsync make build-essential
```

Install Rust using [the official instructions](https://www.rust-lang.org/tools/install). From the checkout:

```sh
rustup component add rustfmt clippy
make build
./bin/fastgpu --version
make install
export PATH="$HOME/.local/bin:$PATH"
```

`make build` builds with `cargo build --release --locked --target-dir target` and copies the result to `bin/fastgpu`. `make install` installs it in `~/.local/bin/fastgpu`. Add the PATH export to your shell configuration. `CARGO=/path/to/cargo` overrides Cargo; `DESTDIR` supplies an installation staging prefix.

The executable uses the system C library and rustls for HTTPS. It needs no Rust runtime or OpenSSL installation. The current implementation uses Unix-specific APIs; Windows support is not implemented. The supplied GPU image and npm archive target Linux x86-64.

### npm archive

The current package is named `fastgpu`, version `0.3.0`, and has `private: true`. This prevents npm registry publication; it does not prevent local archive installation.

```sh
make npm-package
npm install -g ./dist/fastgpu-0.3.0.tgz
command -v fastgpu
fastgpu --version
```

Creating the archive requires Cargo, make, a C toolchain, Node.js, and npm. Installing an existing `.tgz` needs Node/npm but no compiler. Supported Node versions are 22.15+ within 22.x, or 24+. The package supports Linux x64 with glibc, contains the executable, launcher, logo, license, and documentation, and has no npm dependencies or installation scripts.

`npm prefix -g` identifies the installation prefix. If another installation occurs earlier on PATH, your shell may run that copy instead. The [course](how-to.md#30-npm-files-and-lifecycle) explains the launcher and packaging hooks.

### Portable build baseline

A locally compiled executable inherits its build system's glibc requirements. Do not assume that an Ubuntu-built binary runs on every older Linux distribution. The Docker CLI stage uses Debian Bookworm and can supply the archive's executable:

```sh
docker build --target cli -f docker/Dockerfile -t fastgpu-cli-npm .
mkdir -p .local/npm-build
package_container=$(docker create fastgpu-cli-npm)
docker cp "$package_container":/fastgpu .local/npm-build/fastgpu
docker rm "$package_container"
FASTGPU_NPM_BINARY="$PWD/.local/npm-build/fastgpu" make npm-package
```

`FASTGPU_NPM_BINARY` skips compilation in the packaging hook, but that hook still needs Cargo to inspect package metadata. It verifies the package/Cargo/binary versions, Linux x64 ELF header, and executable permissions. It does not prove compatibility with every distribution. Test the archive on the platforms you intend to support; see [validation](#validation-and-current-limits).

## Credentials and first setup

The API key is read in this order:

1. A nonempty `RUNPOD_API_KEY` environment variable.
2. The `apikey` value in `~/.runpod/config.toml`.

The CLI does not have a login command. The key is not stored in the fastgpu profile or sent to the Pod. In Bash, enter it without recording the value in command history:

```bash
read -rs -p 'RunPod API key: ' RUNPOD_API_KEY
printf '\n'
export RUNPOD_API_KEY
```

Use RunPod credentials with access to the operations you intend to perform. Volume mutations require the corresponding write permissions. Keep keys out of project files, examples, and transferred data.

```sh
fastgpu volumes list
fastgpu init --volume ID_OR_NAME --image felipeoperti/fastgpu-workspace:0.3.0
fastgpu up --small --dry-run
fastgpu up --small --skip-model
```

A Network Volume must already exist. If necessary, create one explicitly with [volumes create](#fastgpu-volumes-create). With multiple volumes and no saved or explicit selection, `init`/`up` prompt only in an interactive terminal; otherwise they fail instead of guessing.

`init` generates a dedicated Ed25519 key pair if its selected key path does not exist. By default these files are `id_ed25519` and `id_ed25519.pub` in the profile. The generated key has no passphrase. An existing private key must also have its `.pub` file. For a passphrase-protected key, load it into `ssh-agent` first; automatic connections use batch mode. The agent is not forwarded to the Pod.

## Configuration and state

### Profile location

The profile directory is selected in this order:

1. Nonempty `GPU_CONFIG_DIR`.
2. `$XDG_CONFIG_HOME/gpu` when `XDG_CONFIG_HOME` is set.
3. `~/.config/gpu`.

An independent profile can be selected with:

```sh
export GPU_CONFIG_DIR="$HOME/.config/gpu-experiments"
fastgpu config
```

Profiles do not isolate cloud accounts: creation still checks for other non-terminated `gpu-*` Pods in the account. Use a short profile path because SSH control sockets have operating-system path-length limits.

| File | Contents or purpose |
| --- | --- |
| `config.json` | Effective user settings saved by configuration commands |
| `state.json` | Selected Pod, pending creation name, managed volume, tunnel port |
| `id_ed25519`, `id_ed25519.pub` | Default dedicated SSH identity |
| `ssh_config` | Generated connection to the current Pod |
| `known_hosts` | Recorded SSH server identities |
| `tunnel-POD_ID` | SSH control socket for the managed tunnel |
| `lock` | Exclusive lock for cooperating profile mutations |

The `gpu` profile directory, `GPU_CONFIG_DIR`, remote `gpu` user, and `gpu-*` resource names are intentional compatibility names. The executable is `fastgpu`.

### Settings and defaults

`fastgpu config` prints effective settings as JSON. You can edit `config.json` while no operation is running. JSON comments are not supported. Missing settings receive these defaults; an existing profile may contain different values.

| JSON field | Default | Meaning |
| --- | --- | --- |
| `image` | `felipeoperti/fastgpu-workspace:0.3.0` | Image for newly created Pods |
| `registry` | Empty | RunPod registry credential ID, not the credential itself |
| `volume` | Empty | Saved volume ID/name; otherwise select from account volumes |
| `ssh_key` | `PROFILE/id_ed25519` | Local private key path |
| `min_vram_gb` | `24` | Minimum catalog GPU VRAM |
| `min_ram_gb` | `32` | Minimum system RAM per GPU requested during creation |
| `min_cuda` | `13.0` | Minimum host CUDA version requested from RunPod |
| `container_disk_gb` | `40` | Ephemeral container disk size; minimum 20 GB |
| `max_price` | `0` | Compute-rate cap in USD/hour; nonpositive values disable it |
| `context_tokens` | `131072` | Context passed to the inference engine |
| `local_port` | `8080` | Local loopback port for inference |

`init` saves image/key/registry/volume settings; `volumes use` saves the volume. Flags on `up` and `deploy` override settings for that invocation and do not persist those configuration changes. Operational state and the remote deployment manifest are saved separately.

The context default is not an automatic memory-fit calculation. Choose `--context` and compatible hardware for the specific model. `--min-vram` affects new Pod selection, not the resources of an existing Pod.

### Persistent remote layout

| Path | Purpose |
| --- | --- |
| `/workspace` | Mounted Network Volume |
| `/home/gpu` | Symlink to `/workspace/.gpu/home` |
| `/workspace/.gpu/models` | Managed GGUF presets and verification caches |
| `/workspace/.gpu/ollama/models` | Ollama store |
| `/workspace/.gpu/deployment.json` | Last inference deployment manifest |
| `/workspace/.gpu/logs` | Inference and Supervisor logs |
| `/home/gpu/.cache` | Hugging Face, PyTorch, and uv caches |
| `/workspace/.gpu/hostkeys/POD_ID` | Persistent SSH host identity for each Pod |

The image initializes `progetti`, `checkpoints`, `datasets`, and `.cache` under the home directory. You may create `projects` or any other directories explicitly. Files outside the Network Volume are not retained when the Pod is deleted.

## Command overview

| Command | Main purpose |
| --- | --- |
| `fastgpu`, `help`, `--help`, `--version` | Local help/banner or version |
| `init`, `config` | Save or inspect configuration |
| `up`, `pods`, `status`, `use` | Create/resume, list, inspect, or adopt a Pod |
| `stop`, `down`, `recover` | Stop compute, terminate the managed Pod, or resolve uncertain creation state |
| `shell`, `code` | Remote command/shell or local VS Code on a remote folder |
| `push`, `pull` | Local-to-remote or remote-to-local copies |
| `model pull`, `model list` | Prepare presets or list persistent GGUF files |
| `deploy`, `undeploy` | Start or stop the managed inference service |
| `tunnel`, `connect` | Manage a model tunnel or print connection commands |
| `logs`, `doctor`, `cost`, `info` | Diagnostics and resource information |
| `volumes` | List, show, select, create, rename, resize, or delete Network Volumes |

Every command accepts `-h`/`--help`. The singular `volume` aliases `volumes`. Internal `worker` commands are documented [below](#internal-worker-commands) and hidden from the main help. There is no `mkdir` subcommand; use `shell mkdir -p`.

## Help and configuration commands

### fastgpu help

```sh
fastgpu
fastgpu --help
fastgpu -h
fastgpu help deploy
fastgpu model list --help
fastgpu --version
```

No arguments shows the root help and ASCII logo, just like requesting root help. It does not load the profile or contact RunPod. `--help` shows detailed command descriptions; `-h` is the compact form. `--version` prints `fastgpu version 0.3.0`. Help/version work without credentials, including when the profile JSON is malformed.

### fastgpu init

```text
fastgpu init [--image IMAGE] [--ssh-key SSH_KEY] [--registry REGISTRY] [--volume VOLUME]
```

Select an existing Network Volume, prepare the SSH identity, and save the local configuration. Requires the RunPod API but creates neither a Pod nor a volume.

| Option | Effect |
| --- | --- |
| `--image` | Save a container tag or digest for future Pods |
| `--ssh-key` | Save the local private key path; require its public companion if the private key exists |
| `--registry` | Save a RunPod registry credential ID for a private image; an empty value clears it |
| `--volume` | Resolve an existing volume by ID or unique name; otherwise reuse the saved selector or prompt |

Changing the image does not replace a running Pod. Use the [upgrade procedure](#upgrading-the-image) when the worker version differs.

### fastgpu config

Print the effective local profile as JSON, including defaults. It requires no API key and does not create the profile directory. It prints the SSH key path, not the key contents, and omits the RunPod API key.

## Pod lifecycle

### fastgpu up

```text
fastgpu up [--small | --medium | --large | --xlarge]
           [--volume VOLUME] [--dry-run] [--skip-model]
           [--max-price USD_PER_HOUR] [--min-vram GB] [--image IMAGE] [--disk GB]
```

Create one Secure Cloud On-Demand NVIDIA GPU Pod on the selected volume, or reuse the already selected Pod. Creation requests one GPU, at least four vCPUs, and the profile's minimum system RAM. A stopped selected Pod is started before waiting for SSH. After startup, Qwen is prepared unless `--skip-model` is set. Inference and local-project uploads are separate operations.

| Size | Catalog compute-rate band | Selection within compatible offers |
| --- | --- | --- |
| `--small` (default) | Greater than $0 and below $1.50/hour | Cheapest |
| `--medium` | $1.50 to below $3.50/hour | Closest to $3/hour |
| `--large` | $3.50 through $5/hour | Cheapest |
| `--xlarge` | Above $5/hour | Cheapest |

These are CLI selection bands, not fixed provider prices or VRAM sizes. Offers must satisfy the volume's data center, minimum VRAM, CUDA request, NVIDIA/architecture filters, and any price cap. The source excludes known unsupported older GPU families. There is no automatic move to another data center or size band when capacity is unavailable.

| Option | Detail |
| --- | --- |
| Size flags | Mutually exclusive; affect new Pod selection |
| `--volume` | Override the saved volume for this invocation; refuse if it differs from the selected Pod's volume |
| `--dry-run` | Read API data and print a plan or the selected Pod; do not create/start a Pod or download models |
| `--skip-model` | Skip default Qwen preparation |
| `--max-price` | Filter catalog rates and check actual new-Pod compute cost; does not cap storage or total spend |
| `--min-vram` | Minimum GPU VRAM in GB, default from profile |
| `--image` | Image for a newly created Pod |
| `--disk` | New Pod's container disk in GB, at least 20 |

An existing Pod keeps its GPU, image, disk, and rate even when these creation flags change. Run `down` before creating a replacement. A dry run may reconcile or clear stale local Pod references and acquire a profile lock; it is not a promise of zero local filesystem changes.

New creation saves an operation name before POST and the Pod ID before waiting for startup. A specific capacity rejection can cause a retry with another offer in the same band. An ambiguous write is not repeated automatically. If the actual new-Pod rate exceeds a positive cap, the client attempts to terminate that newly created Pod.

Startup waits up to 25 minutes for a running Pod with a direct SSH endpoint and `/run/gpu/ready`. An error or interruption can leave billable compute running. Inspect `status`/`pods`, then use `down` if you want to release it.

### fastgpu pods

List all account Pods as JSON, including Pods outside the selected profile. Does not select or modify them. Requires API credentials but no selected Pod.

### fastgpu status

With a selected Pod, print `pod` API details and `local` state as JSON. Without a selected Pod, print only local state and make no API request. Use `info --live` for remote system probes.

### fastgpu use

```sh
fastgpu use POD_ID
```

Adopt an existing reachable Pod into this profile. Its name must start with `gpu-`, it must have exactly one Network Volume attachment, and `fastgpu --version` must run through SSH. The command saves the Pod/volume IDs and marks it managed, so subsequent `down` can terminate it. It does not create or restart a Pod, change its image, or change the profile's saved default volume. Worker operations separately require an exact client/worker version match.

### fastgpu stop

Close the managed tunnel and submit RunPod's stop action for the selected managed Pod. Keep local Pod identity so `up` can request a restart. This submits the stop request; it does not wait for the final stopped state. Inspect `status` before restarting if the provider is still transitioning. Restart depends on provider state and available capacity; storage remains separately billable.

### fastgpu down

Terminate the selected managed Pod while retaining its Network Volume. Refuse if the managed flag or expected attachment does not match local state. Close the managed tunnel, request deletion, wait up to two minutes for termination, verify the volume through the API, then clear state.

Container-disk files are lost. Persistent files are retained, but storage continues to incur charges. If the Pod is already absent with a 404 response, local state is cleared; that branch does not recheck the volume. With no selected Pod, the command reports an error instead of choosing another account Pod.

### fastgpu recover

```sh
fastgpu recover
fastgpu pods
# Only after checking the API listing and RunPod console:
fastgpu recover --clear
```

Without `--clear`, print local state without an API request. `pending_name` identifies a creation whose result may have been lost. Inspect account resources and the console first: if the Pod exists and is reachable, adopt it with `use POD_ID`.

`--clear` refuses if a Pod is selected, queries account Pods for the pending name, and refuses while a match exists. Otherwise it resets local state. The flag also confirms that you checked manually; the API may not resolve an uncertain creation immediately. This command does not recover deleted files, restart compute, or delete cloud resources.

## Connections and remote files

### Path rules

| Path supplied to fastgpu | Meaning |
| --- | --- |
| `projects/demo` or `home:projects/demo` | `/home/gpu/projects/demo`, on the volume |
| `vol:datasets` | `/workspace/datasets`, on the volume |
| `/tmp/job` for transfers | Absolute path on the Pod; ephemeral in this example |
| `/workspace/models/model.gguf` for a model | Absolute persistent remote path |

`code` and `shell --cwd` accept relative paths and `home:`/`vol:` prefixes, not absolute paths. `push`/`pull` additionally accept absolute Pod paths. Model paths additionally accept absolute paths under `/workspace` or `/home/gpu`; the worker resolves them and requires them to remain on the volume. Prefixes take relative suffixes: use `vol:models`, not `vol:/models`.

Newlines, NULs, and lexical escapes outside a prefixed root are rejected. File transfers follow normal remote filesystem permissions and symlink behavior; prefixes do not form a security sandbox. Model containment checks are stricter because files must persist.

### fastgpu shell

```text
fastgpu shell [--cwd DIRECTORY] [--tty] [COMMAND ...]
```

Connect as `gpu` to the selected Pod. No command opens an interactive Bash login shell with a terminal. Otherwise run the supplied program, inherit stdin, and propagate its nonzero exit status. The working directory defaults to `home:`. `--tty` requests a terminal for a supplied command.

```sh
fastgpu shell
fastgpu shell mkdir -p projects/demo
fastgpu shell --cwd vol:projects/demo python train.py
fastgpu shell --tty tmux new -A -s work
fastgpu shell bash -lc 'printf "%s\n" "$HOME"; ls -lh /workspace'
```

Put CLI options before the remote executable; later options belong to the remote program. The CLI quotes arguments rather than expanding them as shell code. For pipelines, redirects, variables, or wildcard expansion, explicitly invoke `bash -lc` and quote the script so the local shell does not expand it first.

### fastgpu code

```text
fastgpu code [DIRECTORY]
```

Check that the remote directory exists, then launch local `code --new-window --remote ...`. Default directory: persistent home. Requires local VS Code and the Remote - SSH extension. Install the extension with `code --install-extension ms-vscode-remote.remote-ssh` if needed.

The command adds an `Include` for the profile's generated SSH configuration to `~/.ssh/config`, preserving existing entries. Other fastgpu SSH commands use the profile directly with `ssh -F`. The command does not create the project directory or copy local files.

### fastgpu push

```text
fastgpu push SOURCE DESTINATION
```

Copy a local file/directory into a remote destination directory. Create that directory if necessary. Requires a selected reachable Pod, SSH, and rsync. It works for both persistent paths and absolute Pod-disk paths.

```sh
fastgpu push ./project/ home:projects/demo
fastgpu push ./model.gguf vol:models
fastgpu push ./input.txt /tmp/job
```

### fastgpu pull

```text
fastgpu pull SOURCE DESTINATION
```

Copy a remote file/directory into a local destination directory, creating it if necessary. Requires a reachable Pod even when the source is on the Network Volume. It leaves remote source files in place.

```sh
fastgpu pull home:checkpoints/ ./results
fastgpu pull vol:datasets/sample.json ./downloads
fastgpu pull /tmp/job/output.txt ./results
```

For both transfer commands:

- The destination is a directory, not a requested replacement filename.
- `source/` copies directory contents; `source` copies the directory itself.
- Rsync may update existing same-name files; destination-only files are not deleted.
- Exclusions are `.venv/`, `__pycache__/`, `.env`, `.env.*`, `*.pem`, and `*.key`. There is no CLI flag to override this list.
- Transfers use archive behavior without preserving owner/group, protected arguments, and resumable partial files under `.gpu-rsync-partial`.
- These are explicit one-time copies, not background synchronization. Exclusions do not identify every possible secret filename.

### fastgpu connect

```text
fastgpu connect [--json] [--local-port PORT] [--remote-port PORT]
```

Print the TCP endpoint, direct SSH command, alias-based SSH command, forwarding command, and local endpoint. `--json` gives structured output. Local port defaults to the profile setting; remote port defaults to 8080. Both must be 1–65535. For remote port 8080, also report the `/v1` API URL.

The command refreshes the dedicated SSH configuration, but does not execute the printed commands. If direct SSH is unavailable, it reports that status. `connect --local-port 8888 --remote-port 8888` can print a forwarding command for another remote service.

### fastgpu tunnel

```text
fastgpu tunnel [--local-port PORT]
fastgpu tunnel --close
```

Manage a background SSH forward from `127.0.0.1:LOCAL_PORT` to `127.0.0.1:8080` on the Pod. It does not start inference. Local port defaults to the profile; values must be 1–65535. `--local-port` and `--close` cannot be combined.

Reuse a live managed tunnel on the same port. To change ports, close it first. The implementation uses a control socket under the profile, checks for a busy local port, and requests `ExitOnForwardFailure=yes`. `--close` targets only that managed SSH master; it does not terminate arbitrary SSH sessions or stop compute. Loss of a tunnel does not stop the remote model.

## Models and inference

### fastgpu model

With no subcommand, show model help. Model operations use the selected Pod and require a matching worker version. They do not start a Pod automatically.

### fastgpu model pull

```text
fastgpu model pull [MODEL]
```

Download and verify a supported preset into `/workspace/.gpu/models`. Default: `qwen3.8-27b`. Other supported preset: `nanbeige4.3-33b`. This is a remote model download, unlike top-level `pull`, which copies files to your computer.

| Preset | GGUF filename | Bytes |
| --- | --- | ---: |
| `qwen3.8-27b` | `Qwen3.8-27B-UD-Q4_K_M.gguf` | 16,464,440,224 |
| `nanbeige4.3-33b` | `Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf` | 20,788,825,216 |

The repository, pinned revision, and expected SHA-256 are defined in [src/models.rs](../src/models.rs). Downloads resume through HTTP Range into `.partial` files. Size and checksum must match before the final rename. A complete verified file is reused. Downloading requires the remaining model bytes plus approximately 1 GiB of free volume space. A per-directory lock prevents concurrent managed downloads.

Verification metadata caches size, modification time, and expected hash. This assumes trusted processes on the volume. A checksum error on an existing file does not silently overwrite it; inspect or move that file before retrying.

### fastgpu model list

```text
fastgpu model list [DIRECTORY]
```

Recursively list complete `.gguf` files from `/workspace/.gpu/models` by default. Accept another persistent directory, such as `vol:models` or `/workspace/models`. Print a deploy alias for managed preset files, or a shell-quoted path for custom files, with size in decimal GB. Skip symlinks and non-GGUF/partial files. The directory must exist. Listing is not a checksum verification or a downloadable-model catalog.

### fastgpu deploy

```text
fastgpu deploy [MODEL_OR_PATH] [--engine ENGINE] [--name NAME] [--hf]
               [--tokenizer TOKENIZER] [--modelfile MODELFILE] [--arg VALUE ...]
               [--min-vram GB] [--context TOKENS] [--local-port PORT]
               [--max-price USD_PER_HOUR]
```

Default target: `qwen3.8-27b`; default engine: `llama.cpp`. Validate the request, create/reuse a `small` Pod, check the worker version, and send the deployment manifest through SSH stdin. The worker prepares a preset if needed, verifies persistent inputs, and starts one managed inference service. The client waits for model readiness, opens the local tunnel, and checks readiness through it.

Changing the model, engine, or deployment settings replaces the prior inference service. An identical ready deployment is reused. A failure can leave the Pod running: inspect `logs`, then use `down` when you want to release compute.

| Option | Meaning and constraints |
| --- | --- |
| `MODEL_OR_PATH` | Preset alias, remote persistent model file/directory, or Hugging Face ID with `--hf` |
| `--engine` | `llama.cpp`, `ollama`, or `vllm` |
| `--name` | Served API alias; default derived from the target filename/ID. Must start with a letter/digit and contain only letters, digits, `.`, `_`, `-`, maximum 96 characters |
| `--hf` | vLLM only; interpret target as an `organization/model` Hugging Face ID |
| `--tokenizer` | vLLM only; HF ID or persistent path, required for GGUF |
| `--modelfile` | Ollama only; persistent remote Modelfile |
| `--arg` | Repeatable extra llama.cpp/vLLM argument; use long options, e.g. `--arg=--enforce-eager` |
| `--min-vram` | Minimum VRAM for new Pod selection; does not resize an existing Pod |
| `--context` | 512–262144 tokens, default from profile; memory must fit |
| `--local-port` | Local API loopback port, default from profile |
| `--max-price` | Compute-rate cap for new Pod creation; does not impose a spending limit |

There is no `deploy --small`/`--medium`/`--large` flag. To use another band, explicitly create a Pod with `up` first; deployment reuses it. The CLI never trains or converts a checkpoint into GGUF.

#### llama.cpp

Use a preset or an existing persistent GGUF:

```sh
fastgpu deploy qwen3.8-27b --context 16384
fastgpu deploy vol:models/custom.gguf --name custom --context 4096
fastgpu deploy vol:models/custom.gguf --context 4096 --arg=--temp --arg=0.2
```

The managed command binds remote loopback port 8080, sets the alias and context, requests all GPU layers, disables automatic fitting, selects one parallel sequence, and enables Jinja templates. Model/context fit must be checked for your hardware. List available engine options with `fastgpu shell llama-server --help`.

#### Ollama

```sh
fastgpu deploy vol:models/custom.gguf --engine ollama --name custom --context 4096
```

Ollama starts on remote port 8080 and imports into the persistent store. With no custom file, the worker generates a Modelfile with `FROM` and `PARAMETER num_ctx`. A custom Modelfile controls those model settings itself; make its `FROM` refer to the intended persistent weights. Keep the CLI target consistent with the imported model because that target is fingerprinted.

```text
FROM /workspace/models/custom.gguf
PARAMETER num_ctx 4096
PARAMETER temperature 0.2
```

Save this locally as `Modelfile`, then:

```sh
fastgpu push ./Modelfile vol:models
fastgpu deploy vol:models/custom.gguf --engine ollama --name custom \
  --modelfile vol:models/Modelfile
```

`--arg` is rejected for Ollama. Import can require additional volume space. Supported formats depend on the engine; see [Ollama's import documentation](https://docs.ollama.com/import).

#### vLLM

Use a complete compatible checkpoint, or an HF ID:

```sh
fastgpu deploy home:checkpoints/experiment --engine vllm --name custom \
  --context 4096 --arg=--enforce-eager
fastgpu deploy ORGANIZATION/MODEL --engine vllm --hf --context 4096
```

A local checkpoint must include `config.json`, weights, and the files its tokenizer/architecture requires. HF downloads use the persistent cache. Authentication for gated/private models must be configured separately on the Pod; the local RunPod API key does not authenticate Hugging Face.

For GGUF, specify an explicit tokenizer:

```sh
fastgpu deploy vol:models/custom.gguf --engine vllm \
  --tokenizer ORGANIZATION/TOKENIZER --context 4096
```

GGUF support and compatibility are engine-dependent; consult [vLLM's GGUF documentation](https://docs.vllm.ai/en/latest/features/quantization/gguf/). The live validation recorded below used llama.cpp, not this path.

To pin an HF revision where supported by the installed vLLM version, pass its `--revision` through `--arg`. HF-ID deployments are not content-hashed by fastgpu. Local checkpoint fingerprints track file paths, sizes, and modification times rather than hashing every tensor byte.

#### Extra arguments and readiness

For a flag with a value, use repeated arguments: `--arg=--gpu-memory-utilization --arg=0.8`. CLI-managed flags are reserved, including `--host`, `--port`, `--model`, `--alias`, `--served-model-name`, `--ctx-size`, `--max-model-len`, `--tokenizer`, `--config`, `--api-key`, `--ssl`, `--rpc`, `--download-dir`, `--daemon`, `--help`, and `--version`; SSL-prefixed options and recognized abbreviations of reserved options are rejected too. Short engine flags are rejected; negative numeric values are accepted as values.

For llama.cpp/vLLM, readiness checks health and the expected `/v1/models` alias. For Ollama, the alias must also appear among loaded models. The client allows up to 20 minutes for readiness after the worker deployment call; downloads and Ollama loading have their own waits. These are not a single total-command deadline.

The API base URL is `http://127.0.0.1:LOCAL_PORT/v1`. Requests use the served alias in `model`. The managed service is not exposed as a public RunPod HTTP port, and the CLI does not configure an API key for it. The SSH tunnel binds only local loopback.

### fastgpu undeploy

Close the managed tunnel and stop the inference service on the selected Pod. Keep models, caches, the deployment manifest, and the Pod. Compute remains allocated. The worker version must match the client. Use `down` to terminate compute.

## Diagnostics

### fastgpu logs

```text
fastgpu logs [--follow | --pod]
```

Default: print the last 100 lines of `/workspace/.gpu/logs/inference.log` over SSH. `--follow` follows that file across rotation until interrupted. `--pod` instead reads RunPod's container log event stream with a tail request of 100, useful before SSH is ready. The API stream can stay open until interrupted. The two flags conflict. Before any inference has run, the inference log may not exist.

### fastgpu doctor

```text
fastgpu doctor [--remote]
```

Check local `ssh` and `rsync`, report optional `code`, then verify RunPod API access by listing volumes. `--remote` additionally requires a reachable selected Pod and matching worker, and checks Python, uv, GitHub CLI, Codex CLI, llama.cpp, Ollama, NVIDIA information, PyTorch/vLLM imports, and a small CUDA computation.

The checks stop at the first failure. Installed tools are not automatically logged into their respective accounts. A passing doctor does not prove every model/context fits the GPU.

### fastgpu cost

Print selected Pod ID, status, compute USD/hour, and creation timestamp as JSON. It does not calculate accumulated cost or charge stopped time as if compute were running. Storage is separate; use provider billing for actual charges.

### fastgpu info

```text
fastgpu info [--live]
```

Print a JSON overview with provider Pod details, attached volumes, connection commands, and persistent paths. `--live` adds a remote Python probe for OS, visible CPUs, resolved home, GPU VRAM/usage, driver information, and kernel memory fields.

Credential-bearing environment/registry fields are omitted from provider details. Do not confuse the API Pod's `gpu.memory` field, used here as system RAM, with GPU VRAM; use the live NVIDIA probe for VRAM. `info` still requires a selected Pod.

## Network Volumes

### fastgpu volumes

`fastgpu volumes` defaults to `volumes list`. `fastgpu volume ...` is an alias. Selectors use exact IDs first, otherwise unique names. Duplicate names require an ID. Operations act on account storage; switching a local selection does not move data between volumes or data centers.

### fastgpu volumes list

List account Network Volumes as JSON. No selection or mutation.

### fastgpu volumes show

```text
fastgpu volumes show ID_OR_NAME
```

Resolve one existing volume and print its details. No running Pod is required.

### fastgpu volumes use

```text
fastgpu volumes use ID_OR_NAME
```

Save the resolved volume ID to the profile. Refuse if a selected Pod uses a different volume; run `down` before switching. Does not attach storage to a running Pod or create resources.

### fastgpu volumes create

```text
fastgpu volumes create NAME --size GB --datacenter DATA_CENTER_ID [--type TYPE]
```

Create paid storage explicitly. `--size` must be 10–4096 GB and `--datacenter` must be nonempty. `--type` defaults to `STANDARD`; the other accepted value is `HIGH_PERFORMANCE`. Availability is provider-dependent. The CLI does not offer a data-center-list command: use RunPod's console/API to choose a supported location.

Print the created volume as JSON, without selecting it or starting a Pod. Use `volumes use` afterward if desired. The volume's data center constrains future Pod placement.

### fastgpu volumes rename

```text
fastgpu volumes rename ID_OR_NAME NEW_NAME
```

Set a nonempty display name through the API. The ID, data, capacity, and attachments remain the same. A saved ID keeps identifying the volume after a rename.

### fastgpu volumes resize

```text
fastgpu volumes resize ID_OR_NAME GB
```

Set total capacity, not an increment. The requested size must be at least the current size and no more than 4096 GB. Shrinking is refused. Increasing capacity is a persistent storage-cost change.

### fastgpu volumes delete

```text
fastgpu volumes delete ID_OR_NAME --confirm EXACT_VOLUME_ID
```

Permanently delete an unprotected volume and its data. Confirmation must match the resolved ID, even if selecting by name. Refuse a volume whose ID matches `config.volume` or `state.volume_id`, or a volume attached to any listed account Pod. Configuration commands save resolved IDs; keep IDs in those fields when editing the profile manually so the protection applies. These checks do not create a distributed lock against concurrent external changes; RunPod also decides whether deletion is permitted.

Use `down` when the intent is to remove only compute. This command is the separate explicit operation for deleting persistent storage.

## Remote project workflow

Clone once into the persistent home:

```sh
fastgpu shell mkdir -p projects
fastgpu shell --cwd home:projects git clone https://github.com/YOUR_ACCOUNT/YOUR_REPOSITORY.git
fastgpu code home:projects/YOUR_REPOSITORY
```

GitHub authentication is separate from RunPod/SSH authentication. For a local project, use `push` instead. `up` and `deploy` do not continually recopy your repository.

Create project-specific dependencies on the volume:

```sh
fastgpu shell --cwd home:projects/demo uv venv
fastgpu shell --cwd home:projects/demo uv pip install --python .venv/bin/python -r requirements.txt
fastgpu shell --cwd home:projects/demo .venv/bin/python train.py
```

For a long job, open `fastgpu shell --tty tmux new -A -s training`, then run the script inside tmux. Detach with Ctrl-B followed by D. A tmux session survives a disconnected SSH client, but not Pod termination; checkpoints must still be saved on persistent storage.

Useful location checks:

```sh
fastgpu shell readlink -f /home/gpu
fastgpu shell df -h /workspace
fastgpu shell ls -lh /workspace/.gpu/models
```

On shared filesystems, `df` may describe the storage pool rather than your volume quota. Use the provider's volume details for allocated capacity. One managed environment per volume is supported; a workspace lock protects cooperating fastgpu containers. Processes that ignore it can still modify the same data.

## Container image

The default published worker image is `felipeoperti/fastgpu-workspace:0.3.0`. The image validated on November 15, 2026 has digest:

```text
sha256:71d1d636a162a6c9d9ba02b39f9bdcd768f4cb59549903cc68dea70d46cfe9e7
```

Pin that exact build with:

```sh
fastgpu init --image felipeoperti/fastgpu-workspace@sha256:71d1d636a162a6c9d9ba02b39f9bdcd768f4cb59549903cc68dea70d46cfe9e7
```

The local help-banner change followed this image's publication; it does not change the worker protocol. Version checks compare version strings, while a digest identifies an exact image build.

### Components and extension points

The Dockerfile uses a Rust 1.91.1 Bookworm builder and an Ubuntu 24.04 CUDA 13.0.2 runtime image. The source pins uv 0.13.6, vLLM 0.33.0, llama.cpp b12034, Ollama 0.40.13, GitHub CLI 2.103.3, and Codex CLI 0.166.0. These are repository pins, not claims about the latest upstream versions. Python 3.12, SSH, rsync, tmux, and build tools are also included.

| File | What to customize |
| --- | --- |
| [docker/packages.apt](../docker/packages.apt) | System packages |
| [docker/requirements.txt](../docker/requirements.txt) | Inference Python dependencies |
| [docker/install.d](../docker/install.d) | Numbered installation scripts, executed in order |
| [docker/Dockerfile](../docker/Dockerfile) | Base images, layers, environment, metadata |
| [docker/entrypoint.sh](../docker/entrypoint.sh) | Persistent home, SSH, and startup checks |
| [docker/supervisord.conf](../docker/supervisord.conf) | Services and log rotation |

The final image contains the compiled worker, not the Rust compiler. Project-specific virtual environments can persist on the volume; system changes belong in a rebuilt image. Apt/transitive dependencies may change between builds even when direct dependencies are pinned. `/opt/python-installed.txt` records the resolved Python environment.

Startup requires a mounted `/workspace` and `PUBLIC_KEY`; `up` supplies them. It initializes the managed `.gpu` namespace, checks its format and write access, establishes the persistent home, prepares SSH keys, obtains a workspace lock, and starts Supervisor. The user is `gpu` (UID/GID 2000). Startup does not recursively change ownership of user data. Some volumes remap ownership; sharing storage is not a user-isolation mechanism.

Codex includes the distribution's `bubblewrap` package. Its `~/.codex/app-server-control` directory points to `/run/gpu/codex-app-server-control`, owned by `gpu` with mode `0700`, so daemon startup works when the Network Volume reports permissive directory modes. An existing directory is preserved inside `~/.codex/app-server-control.backup.XXXXXX` before replacement with the symlink. Configuration and sessions remain on the persistent volume; the socket directory is recreated at container startup.

Supervisor starts SSH automatically. Inference starts through `deploy`, including after a new Pod is created from an existing volume. Inference logs rotate at 20 MB with three backups. `HF_HOME`, `TORCH_HOME`, `UV_CACHE_DIR`, and `OLLAMA_MODELS` point to persistent locations. `VLLM_USE_FLASHINFER_SAMPLER=0` is an image setting; it does not disable GPU inference.

### Upgrading the image

Save work on the volume, then explicitly replace compute:

```sh
fastgpu down
fastgpu init --image felipeoperti/fastgpu-workspace:0.3.0
fastgpu up --small --skip-model
fastgpu doctor --remote
```

Only run `down` when a managed Pod is selected. Updating the local CLI or saving a new image reference does not modify an existing Pod. The old 0.2.1 worker is incompatible with current model/deployment commands; the CLI reports the mismatch before those worker operations.

To build and publish your own image, choose a registry you control:

```sh
make image IMAGE=YOUR_ACCOUNT/fastgpu-workspace:0.3.0
docker login
docker push YOUR_ACCOUNT/fastgpu-workspace:0.3.0
```

Publication is an explicit external operation. A local image build is not automatically available to RunPod. Private images require a registry credential configured in RunPod and its ID saved with `init --registry`. Match the worker version to the local client and record the published digest. Public npm publication is a separate workflow described in [the course](how-to.md#31-publishing-a-public-release).

## Internal worker commands

The same executable runs inside the container. These commands assume its initialized filesystem and services; they are not normal local workflows and are hidden from root help.

| Internal command | Action |
| --- | --- |
| `fastgpu worker prepare [MODEL]` | Prepare a preset, default Qwen |
| `fastgpu worker list-models [DIRECTORY]` | List persistent GGUFs, default `/workspace/.gpu/models` |
| `fastgpu worker deploy` | Read a JSON deployment from stdin, validate it, prepare and start the selected engine |
| `fastgpu worker serve` | Read the saved manifest, check its fingerprint, and replace the worker process with the engine |
| `fastgpu worker ready` | Check health/catalog and the expected model |
| `fastgpu worker stop` | Ask Supervisor to stop `inference` |
| `fastgpu worker check` | Run the image installation/GPU diagnostics |

The deployment manifest contains `model`, `engine`, `target`, `context`, `sha256`, and optional `hf`, `tokenizer`, `args`, and `modelfile`. Unknown fields are rejected; stdin is limited to 1 MiB. Worker deployment takes a lock, resolves presets and persistent paths, computes the fingerprint, and reuses an identical ready service. Otherwise it stops the old service, writes the manifest atomically, and starts the new one. Do not edit weights while inference is using them.

## Troubleshooting

| Symptom | Action |
| --- | --- |
| Command missing or old behavior | Check `command -v fastgpu`, `fastgpu --version`, PATH, and native versus npm installations |
| Missing API key | Set `RUNPOD_API_KEY` or configure runpodctl; avoid printing credentials |
| Ambiguous volume | Pass an exact ID to `init --volume` or `volumes use` |
| No compatible GPU | Inspect `up --small --dry-run`, the selected data center, VRAM/CUDA requirements, price cap, and available capacity |
| Another `gpu-*` Pod exists | Inspect `pods`; reuse/adopt it rather than creating a second environment |
| Uncertain creation | Inspect `recover`, `pods`, and the RunPod console; adopt the existing Pod or clear state only after verification |
| Pod running but SSH unavailable | Read `logs --pod`; large image extraction can precede SSH availability |
| Old worker or `accepts 1 arg(s), received 2` from an old image | Upgrade the image and recreate the Pod, retaining the volume |
| Host-key mismatch | Verify the Pod/server identity before changing the profile's `known_hosts` |
| Busy local port | Use `deploy --local-port PORT`, or close the managed tunnel before reopening it on another port |
| `model list` cannot read a directory | Use an existing persistent directory and check access through `shell` |
| Checksum mismatch or insufficient space | Inspect model files and volume capacity; incomplete downloads remain resumable, but corrupt complete files require explicit handling |
| Model fails to become ready | Read `logs`; check format, tokenizer, context, VRAM, and engine options. The Pod may remain running |
| No inference after restart | Run `deploy` again; startup does not automatically launch the saved manifest |
| Transfer omitted a file | Check the exclusions and source trailing slash; destinations are directories |
| VS Code fails | Verify `code` on PATH, Remote - SSH, directory existence, and the generated SSH include |
| Busy profile lock | Let the other operation finish or inspect it; deleting a lock filename does not safely terminate an active operation |
| Old Linux cannot run npm binary | Verify Node, CPU architecture, execute permissions, and glibc compatibility; build against an appropriate baseline |

Errors use the `fastgpu:` prefix. Successful commands exit 0. Child command failures preserve a positive exit code when available; other errors and interrupted operations generally exit 1. SIGINT/SIGTERM cancel the current local operation, but cannot undo a cloud request already accepted. SSH may also leave a remote job running; inspect actual resources after interruption.

## Validation and current limits

The current local suite has **45 Rust tests** and **9 npm tests**. `make test` checks formatting, runs the Rust suite, and runs Clippy; `make test-npm` validates launcher behavior and offline package installation. Help/banner behavior, all command parsers, profile compatibility, API mocks, recovery, protected volumes, transfers, model verification, and readiness are covered by these checks or targeted smoke tests.

Live validation on November 15, 2026 used only the `small` band: one 32 GB RTX PRO 4500 Blackwell, at a provider-reported compute rate of $0.72/hour. It covered configuration, Pod selection/status/info, SSH and an interactive terminal, CUDA computation, VS Code launch, transfer round trips through home/volume/Pod disk, Qwen listing/preparation/deployment, log streams, tunnels, inference shutdown, stop/restart, and Pod termination. Qwen through llama.cpp at a 16,384-token context returned `4` to a 2+2 request over the local tunnel. Repeated deployment succeeded.

Volume create/rename/resize/select/protected-delete/delete checks used a separate temporary volume. The preexisting volume was retained; the test Pod and temporary cloud volume were removed. Docker startup, real SSH, and a small CPU llama.cpp completion were also checked locally. The Docker-built npm archive was tested on Debian Bookworm; glibc symbol inspection recorded a 2.34 minimum for that tested build, not a guarantee for future builds.

These results do not validate every GPU, engine/model combination, long context, or generation feature. Nanbeige and Ollama/vLLM GPU deployment were not revalidated in this pass. VS Code launch was checked rather than a full editor workflow. No medium, large, or xlarge Pod was allocated for these checks.

Current design limits:

- Linux client, one selected Pod and one managed inference service per profile; new creation checks for other `gpu-*` Pods in the account.
- One NVIDIA GPU per Pod, Secure Cloud On-Demand, constrained by the selected Network Volume.
- Local/profile and volume locks cannot make concurrent operations from different machines globally atomic.
- No automatic training, GGUF conversion, continuous sync, S3 backend, multi-GPU orchestration, spend budget, or shutdown timer.
- Stopped Pods have no guaranteed future GPU capacity; retained volumes remain separately billable.
- File verification caches and checkpoint metadata fingerprints assume trusted writers on the volume.

Implementation details are linked throughout this reference. For provider behavior outside the CLI, consult [RunPod's API](https://docs.runpod.io/api-reference-v2/overview) and [Network Volume documentation](https://docs.runpod.io/storage/network-volumes).

# CLI reference

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

The default profile is `~/.config/gpu`; override it with `XDG_CONFIG_HOME` or `GPU_CONFIG_DIR`. API credentials come from `RUNPOD_API_KEY` or runpodctl's TOML configuration. See [installation](INSTALLATION.md) for the full procedure.

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

See [models](MODELS.md) for format selection and complete examples.

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

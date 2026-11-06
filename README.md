# fastgpu

A Go CLI that turns a RunPod GPU machine into an extension of your local computer: SSH, VS Code, persistent repositories and models, file transfers, and inference with **llama.cpp, Ollama, or vLLM**.

Run training with your own scripts from a terminal or VS Code. Once you save a GGUF file or checkpoint on the volume, `fastgpu deploy` makes it available on localhost through an SSH tunnel.

## Installation

You need Linux, OpenSSH, and rsync. Building from source also requires Go 1.24 or later and make. For editing, install VS Code locally with the **Remote - SSH** extension.

From a local copy of the source:

```sh
make install
export PATH="$HOME/.local/bin:$PATH"
fastgpu --version
fastgpu --help
```

`make install` builds the binary and installs it in `~/.local/bin/fastgpu`. Add the PATH line to `~/.bashrc` or `~/.zshrc` to make it permanent. The compiled binary does not require Go at runtime. See [installation and configuration](docs/INSTALLATION.md) for dependencies, keys, and upgrades.

The command is now `fastgpu`, avoiding Oh My Zsh's `gpu` Git alias. The published `felipeoperti/fastgpu-workspace:0.2.1` image contains the renamed executable. Existing profiles keep their previous image reference and need an explicit [image upgrade](docs/INSTALLATION.md#upgrade-the-client-and-image) before remote operations. Existing profile and volume paths are retained.

## Initial setup

Create an API key in your RunPod settings and expose it as `RUNPOD_API_KEY`. Alternatively, the CLI reads an existing runpodctl configuration from `~/.runpod/config.toml`.

```sh
# Replace this placeholder before running; do not store your key in the repository.
export RUNPOD_API_KEY='YOUR_RUNPOD_API_KEY'
fastgpu volumes list
fastgpu init
fastgpu up --dry-run
fastgpu up --skip-model
```

`init` asks which Network Volume to use when several are available and generates a dedicated SSH key. You can set `--volume ID_OR_NAME`, `--ssh-key PATH`, and `--image IMAGE`. If you have no volumes, create one with `fastgpu volumes create`, choosing its size and data center explicitly.

`fastgpu` without arguments starts the default `small` environment, just like `fastgpu up`. Choose a size with `fastgpu up --small`, `--medium`, `--large`, or `--xlarge`. The CLI selects compatible **On-Demand** NVIDIA GPUs available in the volume's data center: small below $1.50/h, medium from $1.50 to below $3.50/h (closest to $3/h), large from $3.50 to $5/h, and xlarge above $5/h. Except for medium, it chooses the cheapest offer in the size's range. If none is available, startup fails. Use `--max-price` to cap compute cost in USD/hour. Storage is billed separately. Running `up` without `--skip-model` also prepares the Qwen preset, a download of about 16.5 GB.

Only one fastgpu Pod may be created at a time. Repeating `up` reports the existing machine and reuses it; run `fastgpu down` before creating a new machine or changing size. Creation also checks for existing fastgpu Pods on the account when no Pod is selected locally.

## Where your data lives

The Network Volume is mounted at **`/workspace`**. The home directory **`/home/gpu`** is a symlink to **`/workspace/.gpu/home`**.

| Data | Persistent location |
| --- | --- |
| Repositories and projects | A directory under `/home/gpu`, such as `~/projects` |
| Datasets | `/home/gpu/datasets` or another directory under `/workspace` |
| Training checkpoints and outputs | `/home/gpu/checkpoints` or another directory under `/workspace` |
| Preset GGUF files | `/workspace/.gpu/models` |
| Hugging Face, PyTorch, and uv caches | `/home/gpu/.cache` |
| Ollama models and blobs | `/workspace/.gpu/ollama/models` |

Create `~/projects` with `fastgpu mkdir projects`, clone repositories there, and save outputs there or in `~/checkpoints`. A new Pod using the same volume will find them again. `deploy` uses the file already on the volume. Ollama may make an initial copy in its blob store, which is also persistent. The image contains the software; model weights stay on the volume.

## Shell, training, and VS Code

```sh
fastgpu shell
fastgpu mkdir projects/experiment
fastgpu push ./experiment/ home:projects/experiment
fastgpu code
fastgpu code projects/experiment
fastgpu shell --cwd home:projects/experiment python train.py
fastgpu pull home:checkpoints/ ./local-checkpoints
```

`fastgpu shell` without arguments opens an interactive SSH session. `fastgpu shell python train.py` runs a script already in the remote home directory; `--cwd` selects another working directory. Inside the shell, you can clone repositories, create uv environments, and run your scripts. Use tmux for long jobs: see [workflow and persistence](docs/PERSISTENCE.md).

`fastgpu code` opens VS Code in the remote home directory. `fastgpu code projects/experiment` opens that project directly.

`home:` refers to the remote home directory and `vol:` to the volume root. Paths without a prefix are relative to the home directory. A trailing slash on a transfer source copies the directory's contents.

## Deploy your own model

Deployment paths are **remote** and must be on the Network Volume.

```sh
# A GGUF file you have already exported from training.
fastgpu deploy /workspace/models/my-model.gguf --name my-model

# The same file with Ollama.
fastgpu deploy /workspace/models/my-model.gguf --engine ollama --name my-model

# A complete Hugging Face checkpoint, including weights and tokenizer.
fastgpu deploy /home/gpu/checkpoints/experiment --engine vllm --name my-model \
  --context 4096 --arg=--enforce-eager
```

The CLI manages one inference service at a time. Switching models or engines stops the previous service. Training and GGUF conversion remain under your control.

```sh
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"my-model","messages":[{"role":"user","content":"Hello!"}],"max_tokens":128}'
```

The included presets are `qwen3.8-27b` and `nanbeige4.3-33b`. Run `fastgpu deploy nanbeige4.3-33b` to download, verify, and serve Nanbeige's Q4_K_M GGUF (about 20.8 GB). Qwen remains the default. At a 128k context, Nanbeige needs more than 24 GB VRAM with the default full GPU offload and FP16 KV cache; select hardware and cache settings accordingly.

`fastgpu model list` shows installed GGUFs in the managed directory, with the alias or quoted path to pass to `deploy`. Use `fastgpu model list /workspace/models` to search another directory on the volume, including its subdirectories.

GGUF with vLLM requires an explicit tokenizer and a compatible model; vLLM's GGUF support is experimental. Use `--arg` for llama.cpp/vLLM options and `--modelfile` for Ollama: see [deployment and customization](docs/MODELS.md).

## Machine information and connections

```sh
fastgpu info
fastgpu info --live
fastgpu connect
fastgpu connect --json
fastgpu connect --local-port 8888 --remote-port 8888
fastgpu status
fastgpu cost
```

`info` shows provider details, volume, RAM, GPU, status, networking, and connection commands. `--live` also reads system details and VRAM over SSH. `connect` prints the TCP endpoint, full SSH command, alias-based command, and a tunnel command. Printed commands are not executed automatically.

## Volume management

```sh
fastgpu volumes list
fastgpu volumes show ID_OR_NAME
fastgpu volumes use ID_OR_NAME
fastgpu volumes create workspace --size 50 --datacenter DATA_CENTER_ID
fastgpu volumes rename ID_OR_NAME new-name
fastgpu volumes resize ID_OR_NAME 100
fastgpu volumes delete ID_OR_NAME --confirm EXACT_ID
```

Replace placeholders with real values. Deletion requires the exact ID and is refused for a selected volume or one attached to a Pod. `fastgpu down` does not delete the Network Volume.

## When you are done

```sh
fastgpu logs
fastgpu undeploy
fastgpu down
```

`undeploy` stops inference while the Pod remains billable. `down` terminates the managed Pod and checks that the volume still exists. Closing SSH, VS Code, or the tunnel does not terminate the Pod. `stop` preserves the Pod's identity, but resuming it depends on GPU availability on its host.

## Documentation

- [Installation, API keys, SSH keys, and SSH configuration](docs/INSTALLATION.md)
- [Repositories, caches, checkpoints, and persistence](docs/PERSISTENCE.md)
- [Custom models and the three inference engines](docs/MODELS.md)
- [Command reference](docs/CLI.md)
- [Extending and rebuilding the Docker image](docs/IMAGE.md)
- [Architecture and limitations](docs/ARCHITECTURE.md)
- [Validation results](docs/VALIDATION.md)
- [Design decisions](DESIGN.md)

Examples are generic. Local configuration, credentials, Pod state, and working logs are not part of the source tree.

## License

fastgpu is licensed under the [GNU General Public License v3.0](LICENSE) (GPL-3.0-only).

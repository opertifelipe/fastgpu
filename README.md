# fastgpu

<img src="logo/fastgpu-logo.svg" alt="fastgpu logo" width="420">

A Rust CLI for using a RunPod GPU as a remote development environment: persistent projects and models, SSH, local VS Code, file transfers, and inference through **llama.cpp, Ollama, or vLLM**.

Run your own training scripts on the Pod, keep outputs on a Network Volume, and serve a saved model through an SSH tunnel on localhost. You control when compute starts and stops.

## Requirements

- Linux; the supplied npm package and GPU image target x86-64.
- OpenSSH client and rsync for remote operations.
- A RunPod account, API credentials, and a Network Volume in a data center with compatible GPU capacity.
- Rust 1.91+, Cargo, a C toolchain, and make to build from source.
- Node.js 22.15+ within the 22.x series, or Node.js 24+, for npm installation. The package requires glibc.
- Optional: VS Code with Remote - SSH. Docker is needed only to build images or use the containerized build workflow.

## Install

From a source checkout:

```sh
make install
export PATH="$HOME/.local/bin:$PATH"
fastgpu --version
fastgpu --help
```

The executable goes to `~/.local/bin/fastgpu`. Add the PATH line to your shell configuration. The native executable needs no Rust or Node runtime after compilation.

Alternatively, create and install a local npm archive:

```sh
make npm-package
npm install -g ./dist/fastgpu-0.3.0.tgz
```

Installing an existing archive needs Node/npm, but no Cargo or compilation. The repository currently sets `private: true`: this workflow does **not** imply that the package is published on the npm registry. See [build compatibility and installation](docs/CLI.md#installation) and the [public release tutorial](docs/how-to.md#31-publishing-a-public-release).

If you have both installations, `command -v fastgpu` shows which one your shell runs. Running `fastgpu` without arguments prints the logo and help; it does not start compute.

## First environment

Supply a RunPod API key through `RUNPOD_API_KEY`, or configure runpodctl's `~/.runpod/config.toml`. To enter a key without putting its value in Bash history:

```bash
read -rs -p 'RunPod API key: ' RUNPOD_API_KEY
printf '\n'
export RUNPOD_API_KEY
```

Then select your existing storage and inspect the startup plan:

```sh
fastgpu volumes list
fastgpu init --volume ID_OR_NAME --image felipeoperti/fastgpu-workspace:0.3.0
fastgpu up --small --dry-run
```

Replace `ID_OR_NAME` with your volume's ID or unique name. If you need new storage, explicitly create it with `fastgpu volumes create workspace --size 50 --datacenter DATA_CENTER_ID`; storage is paid independently of compute.

Start the environment when the plan is suitable:

```sh
fastgpu up --small --skip-model --max-price 1.50
fastgpu doctor --remote
fastgpu shell
```

`small` selects the cheapest compatible offer below $1.50/hour. Other size bands and exact selection rules are in the [CLI reference](docs/CLI.md#fastgpu-up). Availability depends on the volume's data center. `--max-price` caps a new Pod's compute rate, not storage charges or total spend.

`up` reuses the selected Pod. It does not change that Pod's image or GPU. Without `--skip-model`, it also prepares the Qwen preset, approximately 16.5 GB. It does not start inference.

## Persistent files and remote work

The Network Volume is mounted at `/workspace`. The remote home `/home/gpu` points to `/workspace/.gpu/home`; both survive Pod termination.

| Use | Remote location |
| --- | --- |
| Projects, datasets, training outputs | Your directories under `/home/gpu` or `/workspace` |
| Preset GGUF files | `/workspace/.gpu/models` |
| Hugging Face, PyTorch, and uv caches | `/home/gpu/.cache` |
| Ollama store | `/workspace/.gpu/ollama/models` |
| Inference logs and manifest | `/workspace/.gpu/logs`, `/workspace/.gpu/deployment.json` |

```sh
fastgpu shell mkdir -p projects/experiment
fastgpu push ./experiment/ home:projects/experiment
fastgpu code home:projects/experiment
fastgpu shell --cwd home:projects/experiment python train.py
fastgpu pull home:checkpoints/ ./local-checkpoints
```

`push` uploads from this computer; `pull` downloads from the Pod. `home:` refers to the persistent home and `vol:` to the volume root. Transfers also accept absolute paths such as `/tmp/job`, whose files are lost when the Pod is deleted. Source directories ending in `/` copy their contents. Transfers may update same-name files but retain destination-only files; common environment/key files and virtual environments are excluded.

Use `fastgpu shell mkdir -p ...` to create directories. There is no separate `fastgpu mkdir` command. For long training sessions, use [tmux and persistent project environments](docs/CLI.md#remote-project-workflow).

## Serve a model

Use the included Qwen preset:

```sh
fastgpu deploy qwen3.8-27b --context 16384
curl http://127.0.0.1:8080/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{"model":"qwen3.8-27b","messages":[{"role":"user","content":"Hello!"}],"max_tokens":128}'
```

`deploy` creates a `small` Pod if none is selected, prepares the model, starts inference, waits for readiness, and opens the tunnel. The explicit context above matches the live Qwen validation; the profile default is 131,072 tokens, which is not a guarantee that every model fits every eligible GPU.

To serve your own files, copy them to the volume first:

```sh
fastgpu push ./my-model.gguf vol:models
fastgpu deploy vol:models/my-model.gguf --name my-model --context 4096
fastgpu deploy vol:models/my-model.gguf --engine ollama --name my-model --context 4096
fastgpu deploy home:checkpoints/experiment --engine vllm --name my-model --context 4096
```

llama.cpp requires GGUF. vLLM also supports compatible checkpoints and Hugging Face IDs; Ollama imports compatible models into its own persistent store. Engine compatibility and VRAM requirements vary. Only one managed inference service runs at a time; changing the model or engine replaces it.

`fastgpu model list` lists installed GGUFs and their deploy targets. `fastgpu model pull nanbeige4.3-33b` downloads the second preset without starting inference. See [model management and deployment](docs/CLI.md#models-and-inference) for every option, tokenizer requirements, Modelfiles, and model verification.

## Inspect and finish

```sh
fastgpu status
fastgpu info --live
fastgpu connect
fastgpu cost
fastgpu logs
fastgpu undeploy
fastgpu down
```

`status` reports the selected Pod; `info --live` adds hardware and system probes. `connect` prints connection commands without executing them. `cost` reports an hourly compute rate and creation time, not accumulated charges.

`undeploy` stops inference and closes its tunnel while leaving compute running. `stop` requests a stopped Pod that `up` can attempt to resume. **`down` deletes the managed Pod and its container disk, retaining the Network Volume.** Storage remains billable. Closing SSH, VS Code, or a tunnel does not terminate compute.

After an uncertain creation, inspect `fastgpu recover` and `fastgpu pods` before retrying. [Recovery and troubleshooting](docs/CLI.md#troubleshooting) explain how to avoid creating duplicate resources.

## Image upgrades

The published Rust image is `felipeoperti/fastgpu-workspace:0.3.0`. An existing profile keeps its saved image until you change it. Save work on persistent storage, run `fastgpu down`, select the matching image with `fastgpu init --image ...`, and create a new Pod. Updating the local binary alone does not update the remote worker.

See [the image reference and validated digest](docs/CLI.md#container-image) for reproducible selection and custom builds.

## Documentation and development

- [Complete CLI reference](docs/CLI.md): all commands, flags, defaults, storage, recovery, and validation limits.
- [From zero to fastgpu](docs/how-to.md): Rust foundations, implementation walkthrough, SSH tunnels, npm packaging, and public releases.
- [Design decisions](DESIGN.md): architecture and project boundaries.

```sh
cargo run --locked -- --help
make test
make test-npm
```

`make test` runs formatting checks, Rust tests, and Clippy. `make test-npm` validates the launcher and offline archive installation. Tests use local fixtures; live GPU validation is described separately in the manual. Training orchestration, model conversion, continuous synchronization, S3 transfers, and automatic shutdown are outside the current CLI.

## Uninstall

If you want to release a managed Pod, run `fastgpu down` before uninstalling. Remove the installation you used:

```sh
npm uninstall -g fastgpu
# For the native make install location:
rm "$HOME/.local/bin/fastgpu"
```

Uninstalling does not remove the profile, SSH keys, volumes, or remote resources. Keep the profile to manage any environment that remains.

## License

[GNU General Public License v3.0](LICENSE), GPL-3.0-only.

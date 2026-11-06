# Installation and configuration

## Local requirements

- Linux x86-64 for the supplied GPU image. The Go client can be built for other Linux architectures, but these have not all been tested.
- OpenSSH client, `ssh-keygen`, and rsync.
- Go 1.24 or later and make to build from source.
- VS Code with Remote - SSH if you use `fastgpu code`.
- A RunPod account, available credit, and access to the Pod and Network Volume APIs.

On Ubuntu/Debian, install the system tools with:

```sh
sudo apt-get update
sudo apt-get install -y openssh-client rsync make build-essential
```

Install Go using the [official instructions](https://go.dev/doc/install). A C compiler is also needed for race-detector tests; the distributed client is built without CGO. Docker is needed to rebuild the image, not to use the client.

## Build and install

From the project directory:

```sh
make install
export PATH="$HOME/.local/bin:$PATH"
fastgpu --version
fastgpu --help
```

The binary is installed in `~/.local/bin/fastgpu`. To select a compiler, use `make install GO=/path/to/go`. To build only `bin/fastgpu`, run `make build`. Add the PATH export to your shell configuration.

You need a local copy of the source. Publishing it on GitHub is not required to build or use the CLI.

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
fastgpu init
fastgpu up --dry-run
```

When several volumes are available, the first `init` asks which one to select. The saved selection is reused. Use an ID if names are duplicated.

To select these explicitly:

```sh
fastgpu init --volume ID_OR_NAME --image felipeoperti/fastgpu-workspace:0.2.1
```

The published default image is `felipeoperti/fastgpu-workspace:0.2.1` and contains the `fastgpu` executable. The older `gpu-workspace` image contains only the old executable; existing profiles must select the renamed image explicitly. See [upgrading](#upgrade-the-client-and-image), or [build and publish your own image](IMAGE.md) and select it with `--image`.

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

The CLI rename retains `GPU_CONFIG_DIR`, the `gpu` profile directory, `gpu-*` Pod names and SSH aliases, the remote `gpu` user, and `/workspace/.gpu`. Existing profiles, keys, Pod references, and volume contents remain usable without a path migration. The executable name is `fastgpu` both locally and inside the image.

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

Rebuild with `make install`, then check `fastgpu --version`. This installs `~/.local/bin/fastgpu`; an older `gpu` binary is not removed automatically. No change to Oh My Zsh's `gpu` alias is needed.

Images built before the rename contain only the old executable. To upgrade, save your work on the volume, terminate any managed Pod with `fastgpu down`, select the published image with `fastgpu init --image felipeoperti/fastgpu-workspace:0.2.1`, and recreate the environment with `fastgpu up --skip-model`. A saved profile keeps its image reference until you change it explicitly. Updating the local binary alone does not change the remote image.

New deployments require `fastgpu` inside the image with the same version as the client. This prevents new commands from being sent to an older image. Use a Docker digest rather than a mutable tag to select an exact image build.

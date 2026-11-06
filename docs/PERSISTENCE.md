# Persistence, repositories, and training

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

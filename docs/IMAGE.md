# Extending the Docker image

The image contains software and libraries. Repositories, checkpoints, models, and caches live on the Network Volume. The build copies only the required source and installation scripts; keys, local profiles, and logs are excluded from its context.

## Components

- Ubuntu 24.04 with CUDA runtime 13.0.2, requiring a compatible NVIDIA host.
- Python 3.12, uv 0.13.6, and a vLLM 0.33.0 environment at `/opt/vllm`.
- llama.cpp release `b12034`, Linux x64 CUDA 12.8 binaries, and matching runtime libraries at `/opt/llama`.
- Ollama 0.40.13 for Linux x86-64, including libraries from its official distribution.
- Git, GitHub CLI 2.103.3, Codex CLI 0.166.0, OpenSSH server, rsync, tmux, and build tools.
- Python headers and Ninja for compiling Triton components.

The `fastgpu` CLI is compiled from `cmd/fastgpu` in the Go 1.27.3 build stage without CGO and installed at `/usr/local/bin/fastgpu`. Supervisor invokes `fastgpu worker serve`. Main downloaded archives have pinned versions and SHA-256 checksums. Direct Python dependencies are pinned; transitive and apt dependencies can change between builds. `/opt/python-installed.txt` records the resolved Python versions.

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
make image IMAGE=YOUR_ACCOUNT/fastgpu-workspace:0.2.1
```

The image is large because it includes three inference engines and GPU libraries. Allow enough disk space for layers, extraction, and Docker's cache. Docker is not required to use the client alone.

## Registry and image publishing

RunPod must be able to pull the image from a registry. To publish your own:

```sh
docker login
docker push YOUR_ACCOUNT/fastgpu-workspace:0.2.1
docker inspect --format '{{index .RepoDigests 0}}' YOUR_ACCOUNT/fastgpu-workspace:0.2.1
fastgpu init --image YOUR_ACCOUNT/fastgpu-workspace@sha256:DIGEST
```

Replace these placeholders with real references. A digest selects an exact image; use a new tag for each release. For private images, configure a registry credential in RunPod and pass its ID to `fastgpu init --registry`.

Publishing an image does not publish the source on GitHub. The published `felipeoperti/fastgpu-workspace:0.2.1` image contains `fastgpu`. The earlier `gpu-workspace` image contains only the old executable; use the renamed image or build and publish your own image containing `fastgpu` with the same version as the local client. The published digest and local startup checks are recorded in [validation](VALIDATION.md#published-renamed-image--november-2-2026).

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

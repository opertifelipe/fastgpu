# Validation record

This document distinguishes completed tests from functionality that still needs validation. Local operational reports may contain account identifiers and are excluded from repository documentation.

## Previously validated baseline

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

Help, argument placeholders, prompts, messages, errors, worker checks, and container startup messages are now in English. Informational JSON fields from `info` and `connect` are also English; see [the CLI reference](CLI.md#machine-information).

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
fastgpu --help
fastgpu deploy --help
fastgpu volumes --help
fastgpu doctor
```

Tests cover volume and offer selection, pagination, reconciliation after uncertain creation, deletion of only the Pod, quoting, locks, and SSE logs. New tests cover persistent paths, GGUF fingerprints, engine validation, readiness aliases, and information output without credential-bearing containers.

Preset tests cover both Qwen and Nanbeige through local HTTP downloads: resuming partial files, servers ignoring Range, checksum failures, and reuse of verified files. Listing tests cover preset aliases, custom paths with spaces and quotes, directory selection, and exclusion of partial files, metadata, and symlinks. Nanbeige has not yet been validated with real GPU inference, long context, thinking, or tool calls; the live results above describe the earlier Qwen validation.

For live testing, create a dedicated Pod with an explicitly selected volume. Check `doctor --remote`, `info --live`, `connect`, an interactive `shell`, a transfer round trip, VS Code, and generation with each affected engine. Run `undeploy` and `down` afterward, then verify that the volume exists and preexisting resources have not changed.

Tests do not automatically authenticate GitHub/Codex and do not certify every GPU, quantization format, model, or option combination. GGUF deployment with vLLM remains experimental and was not exercised in this validation; the checkpoint path was tested for vLLM. No automatic training was performed, as training management is outside the CLI's scope.

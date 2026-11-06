# Architecture and limitations

## Components

The `fastgpu` Go client uses RunPod REST API v2, OpenSSH, and rsync. Its entry point is `cmd/fastgpu`, with implementation in `internal/fastgpu`. The same executable contains internal `worker` commands used inside the container for downloads, installation checks, and service startup. No additional control server is required.

Main commands are in `cli.go`; APIs and resource selection in `api.go`/`lifecycle.go`; SSH, terminals, VS Code, and transfers in `ssh.go`; connection information in `info.go`; engines in `engines.go`; and the preset and remote checks in `worker.go`.

## Storage

The Network Volume is mounted at `/workspace` and survives Pod termination. The home directory is a symlink to `/workspace/.gpu/home`. This namespace also holds models, the Ollama store, logs, and the deployment manifest. The image supplies software while the volume retains user data.

The volume's data center constrains Pod placement. A container disk or Pod volume disk does not replace this persistence. One managed environment per volume is supported; a lock prevents concurrent startup against the same home directory.

See [persistence](PERSISTENCE.md) for the layout and workflows.

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

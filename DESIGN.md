# Design decisions

## Goal

`fastgpu` turns a RunPod Pod into a personal development and compute environment: persistent repositories and data, SSH access, local VS Code, your own scripts, and model deployment accessible from localhost.

Users run training with their preferred tools. The client manages infrastructure, volume attachment, connections, and the inference service.

## Language

Go provides a single executable without an additional runtime, HTTP APIs, signal handling, and process management with a small implementation. OpenSSH and rsync handle authentication, terminals, tunnels, and transfers. Rust would also suit this kind of client; Go keeps this project's implementation straightforward.

## Image and data separation

The Ubuntu/CUDA image contains software. The Network Volume mounted at `/workspace` contains the home directory, repositories, datasets, checkpoints, GGUF files, and caches. `/home/gpu` points to the home directory on that volume.

This allows Pods to be recreated or images updated while retaining work. System software is added to the image; project dependencies can live in persistent uv environments inside each repository.

## Inference engines

Deployment accepts a path on the volume and selects llama.cpp, Ollama, or vLLM. One managed service uses internal port 8080; the client exposes its API through SSH on the local loopback interface.

Engines have different format and configuration requirements. The CLI provides additional arguments and a custom Modelfile without assuming that every GGUF works with every engine. Qwen and Nanbeige are optional presets; the main workflow supports users' own models.

## Machine selection and control

Selection looks for a compatible On-Demand NVIDIA GPU available in the volume's data center, within the selected size's price range and any configured price cap. Small is the default; medium prefers a rate near $3/hour, while small, large, and xlarge prefer the cheapest matching offer. The actual compute rate is checked after creation. Capacity shortages do not trigger automatic replacement volumes or data migration.

Repeated startup reports and reuses the selected Pod. Before creating a Pod, the client checks the account for other non-terminated fastgpu Pods; a new machine requires explicitly terminating the previous one with `down`.

`info` shows resources and networking; `connect` produces SSH/TCP commands; `shell` opens an interactive session or runs a remote program. API credentials remain local and SSH uses a dedicated identity.

## Future extensions

Extension points include engine definitions, Docker installation scripts, and profile configuration. Continuous synchronization, autonomous shutdown, S3 transfers, and training management are outside the current implementation. Start with the [README](README.md) and see [architecture](docs/ARCHITECTURE.md) for details.

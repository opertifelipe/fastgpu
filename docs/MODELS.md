# Deploying your own models

## Workflow

1. Run training with your own tools over SSH or in VS Code.
2. Save a GGUF file or a complete checkpoint in the persistent home directory or under `/workspace`.
3. Choose an engine and run `fastgpu deploy` from your local computer.
4. Call the model at `http://127.0.0.1:8080/v1`.

Deployment paths refer to the **remote machine**. If a GGUF is on your local computer, copy it first with `fastgpu push ./my-model.gguf vol:models`.

## Formats and engines

| Engine | Main input | Customization |
| --- | --- | --- |
| llama.cpp | GGUF file | Repeat `--arg` for executable options |
| Ollama | GGUF or a supported Safetensors directory | `--modelfile` pointing to a file on the volume |
| vLLM | Hugging Face checkpoint directory or remote model ID | `--arg`, `--tokenizer`, `--hf` |
| vLLM with GGUF | Supported GGUF and an explicit compatible tokenizer | Experimental, model-dependent support |

Engines differ in supported architectures, quantization formats, and templates. The same file is not automatically compatible with all three. The CLI lets you supply the required settings; training and format conversion remain manual.

## llama.cpp

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine llama.cpp --name my-model --context 4096
```

You can also use `home:checkpoints/my-model.gguf`, `vol:models/my-model.gguf`, or `/home/gpu/checkpoints/my-model.gguf`. Unprefixed relative paths are relative to the remote home directory. The resolved path must remain on the Network Volume, including after following symlinks.

By default, the server uses one slot, offloads all layers to the GPU, disables automatic context fitting, and uses the model's Jinja template. To add model-specific options:

```sh
fastgpu deploy /workspace/models/my-model.gguf --name my-model \
  --context 4096 --arg=--threads --arg=8
```

Each `--arg` supplies one argument directly to the process without shell interpretation. Use the `=` form for values starting with `--`. Address, port, primary model path, alias, authentication, and configuration options managed by the CLI cannot be overridden through `--arg`.

`--name` sets the model name in the API. If omitted, the name is derived from the filename, normalized, and stripped of `.gguf`.

## Ollama

Ollama is included in the image. Its server starts when you select this engine:

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine ollama --name my-model --context 4096
```

The CLI starts `ollama serve` on the remote loopback interface, imports the model, and checks that it loads. It generates a Modelfile with `FROM` pointing to the target path and `PARAMETER num_ctx` set to the chosen context.

For custom templates, parameters, or adapters, save a Modelfile on the volume:

```text
FROM /workspace/models/my-model.gguf
PARAMETER num_ctx 4096
PARAMETER temperature 0.6
SYSTEM Answer in English.
```

Then run:

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine ollama --name my-model \
  --modelfile /workspace/models/Modelfile
```

With `--modelfile`, that file controls the content and parameters: set `num_ctx` there too. Keep every artifact referenced by the Modelfile on the volume. Import may create an initial copy in Ollama's storage format; it stays under `/workspace/.gpu/ollama/models`.

Both the OpenAI-compatible API and native endpoints such as `http://127.0.0.1:8080/api/tags` are available through the same tunnel. Ollama may show the model with a `:latest` suffix in its catalog.

## vLLM with checkpoints

An exported checkpoint must contain the required configuration, weights, and tokenizer. For LoRA/QLoRA, prepare a complete checkpoint or supply the vLLM options needed for your setup.

```sh
fastgpu deploy /home/gpu/checkpoints/experiment --engine vllm --name my-model \
  --context 4096 --arg=--enforce-eager \
  --arg=--gpu-memory-utilization --arg=0.85
```

For a model from Hugging Face:

```sh
fastgpu deploy ORGANIZATION/MODEL --engine vllm --hf --name my-model \
  --context 4096 --arg=--revision --arg=REVISION
```

`--hf` distinguishes a remote model ID from a path. Downloads are cached in the persistent home directory. Private and gated models require Hugging Face authentication on the Pod; the CLI does not automatically copy tokens from your local computer.

The image defaults to the native sampler so sampling does not depend on compiling FlashInfer with `nvcc`. Other kernels or models may require additional image dependencies.

## vLLM with GGUF

The [vLLM documentation](https://docs.vllm.ai/en/latest/features/quantization/gguf/) describes GGUF support as experimental. Specify the original model's tokenizer or a compatible persistent tokenizer path:

```sh
fastgpu deploy /workspace/models/my-model.gguf --engine vllm --name my-model \
  --tokenizer ORGANIZATION/BASE_MODEL --context 4096 \
  --arg=--enforce-eager
```

If the model also requires a separate HF configuration, supply it with `--arg=--hf-config-path --arg=/workspace/models/configuration`. Check that your vLLM version and GPU support the architecture and quantization. If they do not, use the HF checkpoint or a supported GGUF with llama.cpp/Ollama.

## Included presets

```sh
fastgpu deploy qwen3.8-27b
fastgpu deploy nanbeige4.3-33b
```

| Deploy alias | GGUF | Download size |
| --- | --- | --- |
| `qwen3.8-27b` | Unsloth `Qwen3.8-27B-UD-Q4_K_M.gguf` | 16.46 GB |
| `nanbeige4.3-33b` | Bartowski `Nanbeige_Nanbeige4.3-33B-Q4_K_M.gguf` | 20.79 GB |

Each preset downloads to `/workspace/.gpu/models` if absent, verifies its pinned size and SHA-256, and uses llama.cpp by default. Later deployments reuse the file. `fastgpu up` and `fastgpu deploy` without a model still select Qwen. Other engines have their own format, template, and tokenizer requirements.

The default context is 131,072 tokens unless overridden by the saved profile or `--context`. Nanbeige's [configuration](https://huggingface.co/Nanbeige/Nanbeige4.3-33B/blob/main/config.json) implies about 26 GiB of FP16 KV cache at that context, in addition to the weights and runtime buffers. A 24 GB GPU is insufficient for full GPU offload at 128k with that cache; choose a larger GPU or suitable cache settings. `--min-vram` only affects creation of a new Pod. Nanbeige generation and tool calling still require validation on the selected GPU and image.

Both the local CLI and the worker in the Docker image must contain the new preset and listing commands. Rebuild the client and image when upgrading from a build that only supports Qwen; see [image builds](IMAGE.md).

## List installed GGUFs

```sh
fastgpu model list
fastgpu model list /workspace/models
fastgpu model list home:checkpoints
```

With no directory, the command searches `/workspace/.gpu/models`. An explicit directory selects another location on the volume. Subdirectories are included; symlinks, partial downloads, and verification metadata are excluded. A reachable Pod is required.

The `DEPLOY TARGET` column shows the short alias for presets in the managed directory and a shell-quoted absolute path for other GGUFs. Copy that value after `fastgpu deploy`. The listing covers GGUF files; checkpoint directories and Ollama's imported blob store are separate storage formats.

## Endpoints and engine changes

```sh
fastgpu deploy /workspace/models/my-model.gguf --name my-model --local-port 8081
curl http://127.0.0.1:8081/v1/models
```

Inference listens on the remote loopback interface. RunPod exposes SSH, and the tunnel makes the API available only on your local loopback interface. `--local-port` changes the local port; the managed remote port remains 8080.

The CLI manages one inference service per Pod. A different deployment stops the previous service, waits for the new engine, and opens the tunnel. Switching can interrupt requests in progress. An identical deployment that is already ready reuses the service.

Do not modify weights while they are being served. Finish exporting to a new file, then deploy that path. GGUF verification caches SHA-256 using file size and modification time; directories are identified using file metadata. The volume is assumed to be modified by trusted processes.

```sh
fastgpu logs
fastgpu logs --follow
fastgpu undeploy
```

`undeploy` stops the service and closes the tunnel. Use `fastgpu down` to terminate the GPU Pod as well. Model or memory errors leave the Pod available for inspection; they do not automatically end billing.

## Sources

- [llama.cpp server](https://github.com/ggml-org/llama.cpp/tree/master/tools/server)
- [Importing models in Ollama](https://docs.ollama.com/import)
- [Ollama Modelfile format](https://docs.ollama.com/modelfile)
- [Ollama API compatibility](https://docs.ollama.com/api/openai-compatibility)
- [GGUF models in vLLM](https://docs.vllm.ai/en/latest/features/quantization/gguf/)

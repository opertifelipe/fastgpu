# syntax=docker/dockerfile:1
ARG CUDA_VERSION=13.0.2
FROM golang:1.27.3-bookworm AS cli
WORKDIR /src
COPY go.mod go.sum ./
RUN go mod download
COPY cmd ./cmd
COPY internal ./internal
RUN CGO_ENABLED=0 go build -trimpath -ldflags='-s -w' -o /fastgpu ./cmd/fastgpu

FROM ghcr.io/astral-sh/uv:0.13.6 AS uv
FROM nvidia/cuda:${CUDA_VERSION}-runtime-ubuntu24.04
ENV DEBIAN_FRONTEND=noninteractive \
    PATH=/opt/vllm/bin:/opt/llama:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    LD_LIBRARY_PATH=/opt/llama:/usr/local/cuda/lib64 \
    PYTHONUNBUFFERED=1 UV_LINK_MODE=copy
COPY docker/packages.apt /tmp/packages.apt
RUN apt-get update && sed '/^[[:space:]]*#/d; /^[[:space:]]*$/d' /tmp/packages.apt | xargs apt-get install -y --no-install-recommends && rm -rf /var/lib/apt/lists/* /tmp/packages.apt
COPY --from=uv /uv /uvx /usr/local/bin/
COPY docker/requirements.txt /tmp/requirements.txt
RUN uv venv --python /usr/bin/python3 /opt/vllm && uv pip install --python /opt/vllm/bin/python --no-cache -r /tmp/requirements.txt && uv pip freeze --python /opt/vllm/bin/python > /opt/python-installed.txt
COPY docker/install.d /opt/gpu/install.d
RUN for script in /opt/gpu/install.d/*.sh; do bash "$script"; done
COPY --from=cli /fastgpu /usr/local/bin/fastgpu
RUN groupadd --gid 2000 gpu && useradd --uid 2000 --gid 2000 --home-dir /home/gpu --shell /bin/bash gpu \
    && passwd -d gpu && mkdir -p /run/sshd /etc/supervisor /opt/gpu
COPY docker/supervisord.conf /etc/supervisor/supervisord.conf
COPY docker/entrypoint.sh /usr/local/bin/fastgpu-entrypoint
RUN chmod 755 /usr/local/bin/fastgpu-entrypoint
ENV VLLM_USE_FLASHINFER_SAMPLER=0 \
    HF_HOME=/home/gpu/.cache/huggingface \
    TORCH_HOME=/home/gpu/.cache/torch \
    UV_CACHE_DIR=/home/gpu/.cache/uv \
    OLLAMA_HOST=127.0.0.1:8080 \
    OLLAMA_MODELS=/workspace/.gpu/ollama/models
LABEL org.opencontainers.image.title="fastgpu-workspace" \
      org.opencontainers.image.source="https://github.com/opertifelipe/gpu-extension" \
      org.opencontainers.image.version="0.2.1"
EXPOSE 22
ENTRYPOINT ["/usr/local/bin/fastgpu-entrypoint"]

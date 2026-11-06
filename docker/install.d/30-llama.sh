#!/usr/bin/env bash
set -euo pipefail
release=b12034
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
curl -fsSL --retry 3 "https://github.com/ggml-org/llama.cpp/releases/download/${release}/llama-${release}-bin-ubuntu-cuda-12.8-x64.tar.gz" -o llama.tar.gz
curl -fsSL --retry 3 "https://github.com/ggml-org/llama.cpp/releases/download/${release}/cudart-llama-${release}-bin-ubuntu-cuda-12.8-x64.tar.gz" -o cudart.tar.gz
printf '%s\n' \
  'aadc322fdf95b50e5c5442fee71b8f757ea31f88ef53cf3dddea877da59d2052  llama.tar.gz' \
  '4cdd72a0052d6796e070c80c23537477ed5b2d1554273945df63533e4c004468  cudart.tar.gz' | sha256sum --check --strict
mkdir -p /opt/llama
tar -xzf llama.tar.gz --strip-components=1 -C /opt/llama
tar -xzf cudart.tar.gz --strip-components=1 -C /opt/llama

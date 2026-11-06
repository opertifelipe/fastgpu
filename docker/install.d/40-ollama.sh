#!/usr/bin/env bash
set -euo pipefail
version=0.40.13
apt-get update
apt-get install -y --no-install-recommends zstd
rm -rf /var/lib/apt/lists/*
archive=$(mktemp)
trap 'rm -f "$archive"' EXIT
curl -fsSL --retry 3 "https://github.com/ollama/ollama/releases/download/v${version}/ollama-linux-amd64.tar.zst" -o "$archive"
printf '%s  %s\n' c9490e0f520be21271d969e6703f012268b1d39f8b70dd58854e5bfef7a75bb6 "$archive" | sha256sum --check --strict
tar --zstd -xf "$archive" -C /usr

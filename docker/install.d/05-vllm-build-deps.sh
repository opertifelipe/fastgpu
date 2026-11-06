#!/usr/bin/env bash
set -euo pipefail
# Triton compiles the Python driver on first GPU use.
apt-get update
apt-get install -y --no-install-recommends python3-dev ninja-build
rm -rf /var/lib/apt/lists/*

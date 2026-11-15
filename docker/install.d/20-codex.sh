#!/usr/bin/env bash
set -euo pipefail
apt-get update
apt-get install -y --no-install-recommends bubblewrap
rm -rf /var/lib/apt/lists/*
npm install --global @openai/codex@0.166.0
npm cache clean --force

#!/usr/bin/env bash
set -euo pipefail
version=2.103.3
archive="gh_${version}_linux_amd64.tar.gz"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
cd "$work"
curl -fsSLO "https://github.com/cli/cli/releases/download/v${version}/${archive}"
curl -fsSLO "https://github.com/cli/cli/releases/download/v${version}/gh_${version}_checksums.txt"
awk -v file="$archive" '$2 == file { print }' "gh_${version}_checksums.txt" | sha256sum --check --strict
tar -xzf "$archive"
install -m 755 "gh_${version}_linux_amd64/bin/gh" /usr/local/bin/gh

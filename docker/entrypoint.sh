#!/usr/bin/env bash
set -euo pipefail
umask 022
mountpoint -q /workspace || { echo 'Network Volume is not mounted at /workspace' >&2; exit 1; }
test -n "${PUBLIC_KEY:-}" || { echo 'PUBLIC_KEY is missing' >&2; exit 1; }

# All managed data uses a dedicated namespace on the existing volume.
state=/workspace/.gpu
mkdir -p "$state"
if [[ -e "$state/format" ]]; then
    [[ "$(cat "$state/format")" == 1 ]] || { echo 'Incompatible fastgpu volume format' >&2; exit 1; }
elif [[ -n "$(ls -A "$state")" ]]; then
    echo '/workspace/.gpu already exists and was not initialized by fastgpu' >&2
    exit 1
else
    printf '1\n' > "$state/format"
fi
for dir in "$state" "$state/home" "$state/models" "$state/logs" "$state/ollama"; do
    mkdir -p "$dir"
    # Some RunPod Network Volumes remap UID/GID and reject chown.
    if ! chown gpu:gpu "$dir" 2>/dev/null; then
        runuser -u gpu -- test -w "$dir" || { echo "Directory is not writable by gpu: $dir" >&2; exit 1; }
    fi
done
# Prevent two fastgpu containers from modifying the same home directory concurrently.
exec 9>"$state/workspace.lock"
flock -n 9 || { echo 'Another fastgpu environment is already using this volume' >&2; exit 1; }
[[ ! -e /home/gpu || -L /home/gpu ]] || { echo '/home/gpu must be a symlink' >&2; exit 1; }
ln -sfn "$state/home" /home/gpu
runuser -u gpu -- mkdir -p /home/gpu/progetti /home/gpu/checkpoints /home/gpu/datasets /home/gpu/.cache
# SSH authorized keys stay on the local disk, which enforces POSIX permissions.
install -d -m 755 /etc/ssh/authorized_keys
printf '%s\n' "$PUBLIC_KEY" > /etc/ssh/authorized_keys/gpu
chmod 644 /etc/ssh/authorized_keys/gpu
if [[ ! -f /home/gpu/.profile ]]; then
    runuser -u gpu -- cp /etc/skel/.profile /home/gpu/.profile
    runuser -u gpu -- cp /etc/skel/.bashrc /home/gpu/.bashrc
fi
cat > /etc/profile.d/gpu.sh <<'PROFILE'
export PATH=/opt/vllm/bin:/opt/llama:/usr/local/bin:$PATH
export LD_LIBRARY_PATH=/opt/llama:/usr/local/cuda/lib64${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}
export VLLM_USE_FLASHINFER_SAMPLER=${VLLM_USE_FLASHINFER_SAMPLER:-0}
export HF_HOME=${HF_HOME:-/home/gpu/.cache/huggingface}
export TORCH_HOME=${TORCH_HOME:-/home/gpu/.cache/torch}
export UV_CACHE_DIR=${UV_CACHE_DIR:-/home/gpu/.cache/uv}
export OLLAMA_HOST=127.0.0.1:8080
export OLLAMA_MODELS=/workspace/.gpu/ollama/models
PROFILE
mkdir -p /run/gpu /run/sshd
chown gpu:gpu /run/gpu
pod_identity=${RUNPOD_POD_ID:-${GPU_WORKSPACE_ID:-local}}
[[ "$pod_identity" =~ ^[A-Za-z0-9_-]+$ ]] || { echo 'Invalid Pod identity' >&2; exit 1; }
hostkeys="$state/hostkeys/$pod_identity"
mkdir -p "$hostkeys"
install -d -m 700 /etc/ssh/gpu-hostkeys
local_key=/etc/ssh/gpu-hostkeys/ssh_host_ed25519_key
if [[ ! -f "$hostkeys/ssh_host_ed25519_key" ]]; then
    ssh-keygen -q -t ed25519 -N '' -f "$local_key"
    cp "$local_key" "$hostkeys/ssh_host_ed25519_key"
    cp "$local_key.pub" "$hostkeys/ssh_host_ed25519_key.pub"
else
    cp "$hostkeys/ssh_host_ed25519_key" "$local_key"
fi
chmod 600 "$local_key"
cat > /etc/ssh/sshd_config.d/gpu.conf <<'SSH'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin no
PubkeyAuthentication yes
AuthorizedKeysFile /etc/ssh/authorized_keys/%u
AllowUsers gpu
AllowTcpForwarding yes
SSH
printf 'HostKey %s\n' "$local_key" >> /etc/ssh/sshd_config.d/gpu.conf
touch /run/gpu/ready
exec /usr/bin/supervisord -n -c /etc/supervisor/supervisord.conf

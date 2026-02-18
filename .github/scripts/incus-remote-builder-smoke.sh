#!/usr/bin/env bash
set -euo pipefail

INCUS_BIN="${INCUS_BIN:-incus}"
MONITOR_PORT="${MONITOR_PORT:-8080}"
WORKDIR="${GITHUB_WORKSPACE:-$PWD}"
ARTIFACT_DIR="${WORKDIR}/artifacts"

RUN_TOKEN="${GITHUB_RUN_ID:-local}-$(date +%s)-$RANDOM"
BUILDER="builder-${RUN_TOKEN}"
CLIENT="client-${RUN_TOKEN}"
MARKER="INCUS_REMOTE_BUILD_MARKER_${RUN_TOKEN}"

mkdir -p "${ARTIFACT_DIR}"

log() {
  printf '[incus-smoke] [%s] %s\n' "$(date -u +%H:%M:%S)" "$*"
}

cleanup() {
  set +e

  if ${INCUS_BIN} info "${BUILDER}" >/dev/null 2>&1; then
    ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'journalctl -u nix-daemon --no-pager -n 300 > /root/nix-daemon.journal.log || true'
    ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'journalctl --no-pager -n 300 > /root/system.journal.log || true'
    ${INCUS_BIN} file pull "${BUILDER}/root/nix-daemon.journal.log" "${ARTIFACT_DIR}/builder-nix-daemon.journal.log" >/dev/null 2>&1 || true
    ${INCUS_BIN} file pull "${BUILDER}/root/system.journal.log" "${ARTIFACT_DIR}/builder-system.journal.log" >/dev/null 2>&1 || true
    ${INCUS_BIN} file pull "${BUILDER}/var/log/nom-output.log" "${ARTIFACT_DIR}/builder-nom-output.log" >/dev/null 2>&1 || true
    ${INCUS_BIN} file pull "${BUILDER}/var/log/nom-forwarder.log" "${ARTIFACT_DIR}/builder-nom-forwarder.log" >/dev/null 2>&1 || true
    ${INCUS_BIN} file pull "${BUILDER}/var/log/nixos-builder-mon.log" "${ARTIFACT_DIR}/builder-monitor.log" >/dev/null 2>&1 || true
    # Capture systemd journal for our transient units
    ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'journalctl -u nom-forwarder --no-pager -n 100 2>/dev/null || true' > "${ARTIFACT_DIR}/builder-nom-forwarder-journal.log" 2>&1 || true
    ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'journalctl -u nixos-buildermon --no-pager -n 100 2>/dev/null || true' > "${ARTIFACT_DIR}/builder-monitor-journal.log" 2>&1 || true
    # Capture dx build output layout for debugging
    ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'find /root/nixos-builder-mon/target/dx -maxdepth 5 2>/dev/null | sort || true' > "${ARTIFACT_DIR}/dist-layout.txt" 2>&1 || true
  fi

  if ${INCUS_BIN} info "${CLIENT}" >/dev/null 2>&1; then
    ${INCUS_BIN} exec "${CLIENT}" -- /bin/sh -lc 'journalctl --no-pager -n 200 > /root/system.journal.log || true'
    ${INCUS_BIN} file pull "${CLIENT}/root/system.journal.log" "${ARTIFACT_DIR}/client-system.journal.log" >/dev/null 2>&1 || true
  fi

  ${INCUS_BIN} stop --force "${CLIENT}" >/dev/null 2>&1 || true
  ${INCUS_BIN} stop --force "${BUILDER}" >/dev/null 2>&1 || true
  ${INCUS_BIN} delete "${CLIENT}" >/dev/null 2>&1 || true
  ${INCUS_BIN} delete "${BUILDER}" >/dev/null 2>&1 || true
}

# Heartbeat so CI logs show the script is alive and where it is in time.
# GitHub Actions does not expose live logs via the API; without this
# a silent hang looks identical to a slow-but-working build.
( while true; do
    printf '[incus-smoke] [%s] heartbeat - script still running\n' "$(date -u +%H:%M:%S)"
    sleep 30
  done ) &
HEARTBEAT_PID=$!

trap 'kill "${HEARTBEAT_PID}" 2>/dev/null; cleanup' EXIT

ensure_dep() {
  local name="$1"
  if ! command -v "${name}" >/dev/null 2>&1; then
    log "Missing dependency: ${name}"
    exit 1
  fi
}

ensure_dep git
ensure_dep jq
ensure_dep curl

if ! ${INCUS_BIN} info >/dev/null 2>&1; then
  log "Incus does not appear initialized; running incus admin init --auto"
  ${INCUS_BIN} admin init --auto
fi

# Ensure a default storage pool and root disk on the default profile
if ! ${INCUS_BIN} storage show default >/dev/null 2>&1; then
  log "Creating default storage pool"
  ${INCUS_BIN} storage create default dir >/dev/null
fi

if ! ${INCUS_BIN} profile device get default root pool >/dev/null 2>&1; then
  log "Adding root disk to default profile"
  ${INCUS_BIN} profile device add default root disk path=/ pool=default >/dev/null
fi

# Ensure a basic bridge network and attach it to the default profile
if ! ${INCUS_BIN} network show incusbr0 >/dev/null 2>&1; then
  log "Creating default bridge network incusbr0"
  ${INCUS_BIN} network create incusbr0 >/dev/null
fi

# Configure the bridge to use host internet access
${INCUS_BIN} network set incusbr0 ipv4.nat true >/dev/null 2>&1 || true
${INCUS_BIN} network set incusbr0 ipv6.nat true >/dev/null 2>&1 || true
${INCUS_BIN} network set incusbr0 dns.mode=dynamic >/dev/null 2>&1 || true

# Enable IP forwarding on the host (required for NAT)
sysctl net.ipv4.ip_forward=1 >/dev/null 2>&1 || true
sysctl net.ipv6.conf.all.forwarding=1 >/dev/null 2>&1 || true

# Add explicit iptables rules for masquerading if NAT isn't working
# Get the primary outgoing interface
PRIMARY_IF=$(ip route show default | awk '/default/ {print $5}' | head -1)
log "Primary outgoing interface: ${PRIMARY_IF}"
if [[ -n "${PRIMARY_IF}" ]]; then
  # Add masquerade rule for all private ranges going through primary interface
  sudo iptables -t nat -A POSTROUTING -s 10.0.0.0/8 -o "${PRIMARY_IF}" -j MASQUERADE 2>/dev/null || true
  sudo iptables -t nat -A POSTROUTING -s 172.16.0.0/12 -o "${PRIMARY_IF}" -j MASQUERADE 2>/dev/null || true
  sudo iptables -t nat -A POSTROUTING -s 192.168.0.0/16 -o "${PRIMARY_IF}" -j MASQUERADE 2>/dev/null || true
  
  # Allow forwarding traffic both directions
  sudo iptables -A FORWARD -i incusbr0 -o "${PRIMARY_IF}" -j ACCEPT 2>/dev/null || true
  sudo iptables -A FORWARD -i "${PRIMARY_IF}" -o incusbr0 -m state --state ESTABLISHED,RELATED -j ACCEPT 2>/dev/null || true
  sudo iptables -A FORWARD -i incusbr0 -j ACCEPT 2>/dev/null || true
  sudo iptables -A FORWARD -o incusbr0 -j ACCEPT 2>/dev/null || true
fi

log "Current iptables NAT rules (POSTROUTING):"
sudo iptables -t nat -L POSTROUTING -n -v 2>/dev/null || true
log "Current iptables filter rules (FORWARD):"
sudo iptables -L FORWARD -n -v 2>/dev/null || true
log "IP forwarding status:"
sysctl net.ipv4.ip_forward 2>/dev/null || true

if ! ${INCUS_BIN} profile device get default eth0 network >/dev/null 2>&1; then
  log "Adding bridged NIC to default profile"
  ${INCUS_BIN} profile device add default eth0 nic network=incusbr0 >/dev/null
fi

# Always ensure the images remote matches what we expect
${INCUS_BIN} remote remove --force images >/dev/null 2>&1 || ${INCUS_BIN} remote remove images >/dev/null 2>&1 || true
log "Adding images remote"
${INCUS_BIN} remote add images https://images.linuxcontainers.org --protocol=simplestreams --public

select_image() {
  local candidate
  for candidate in \
    "images:nixos/24.11/cloud" \
    "images:nixos/24.11/default" \
    "images:nixos/24.11" \
    "images:nixos/25.05/cloud" \
    "images:nixos/25.05/default" \
    "images:nixos/25.05" \
    "images:nixos/25.11/cloud" \
    "images:nixos/25.11/default" \
    "images:nixos/25.11" \
    "images:nixos/unstable/cloud" \
    "images:nixos/unstable/default" \
    "images:nixos/unstable"; do
    if ${INCUS_BIN} image info "${candidate}" >/dev/null 2>&1; then
      printf '%s\n' "${candidate}"
      return 0
    fi
  done
  return 1
}

IMAGE="$(select_image || true)"
if [[ -z "${IMAGE}" ]]; then
  log "Unable to find a NixOS cloud image in Incus images remote"
  exit 1
fi
log "Using image: ${IMAGE}"

# Force VM launch to ensure proper isolation and write access to /nix/store
LAUNCH_OPTS=(--vm)

log "Launching builder VM"
${INCUS_BIN} launch "${IMAGE}" "${BUILDER}" "${LAUNCH_OPTS[@]}" -c limits.cpu=4 -c limits.memory=8GiB -c security.secureboot=false

log "Launching client VM"
${INCUS_BIN} launch "${IMAGE}" "${CLIENT}" "${LAUNCH_OPTS[@]}" -c limits.cpu=2 -c limits.memory=4GiB -c security.secureboot=false

log "Waiting for Incus agent in both VMs"
wait_for_agent() {
  local inst="$1"
  local attempts=60
  for _ in $(seq 1 ${attempts}); do
    if ${INCUS_BIN} exec "${inst}" -- true >/dev/null 2>&1; then
      return 0
    fi
    sleep 5
  done
  log "Incus agent did not become ready for ${inst}"
  exit 1
}

wait_for_agent "${BUILDER}"
wait_for_agent "${CLIENT}"

log "Waiting for network connectivity in both VMs"
wait_for_network() {
  local inst="$1"
  local attempts=30
  for _ in $(seq 1 ${attempts}); do
    if ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc "ip route get 1.1.1.1 >/dev/null 2>&1 && (ping -c 1 -W 2 8.8.8.8 >/dev/null 2>&1 || curl -s --max-time 3 http://example.com >/dev/null 2>&1)"; then
      return 0
    fi
    log "Waiting for network on ${inst}..."
    sleep 2
  done
  log "Network not available for ${inst}"
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'ip addr show 2>/dev/null || true'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'ip route show 2>/dev/null || true'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'ping -c 1 -W 2 8.8.8.8 2>&1 || echo "Ping failed"'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'curl -s --max-time 3 https://cache.nixos.org/nix-cache-info 2>&1 || echo "Curl to cache failed"'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'host cache.nixos.org 8.8.8.8 2>&1 || echo "DNS resolution failed"'
  exit 1
}

wait_for_network "${BUILDER}"
wait_for_network "${CLIENT}"

log "Configuring DNS for VMs"
for inst in "${BUILDER}" "${CLIENT}"; do
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc "cat > /etc/resolv.conf <<'EOF'
nameserver 8.8.8.8
nameserver 1.1.1.1
nameserver 9.9.9.9
EOF
chattr +i /etc/resolv.conf 2>/dev/null || true" || true
done

NIX_FLAGS="--extra-experimental-features flakes --extra-experimental-features nix-command --accept-flake-config --option flake-registry '' --option connect-timeout 120 --option stalled-download-timeout 120 --max-jobs 2"

get_ipv4() {
  local vm="$1"
  ${INCUS_BIN} list "${vm}" --format json | jq -r '.[0].state.network | to_entries[] | select(.key != "lo") | .value.addresses[] | select(.family == "inet") | .address' | sed -n '1p'
}

BUILDER_IP=""
CLIENT_IP=""
for _ in $(seq 1 30); do
  BUILDER_IP="$(get_ipv4 "${BUILDER}" || true)"
  CLIENT_IP="$(get_ipv4 "${CLIENT}" || true)"
  if [[ -n "${BUILDER_IP}" && -n "${CLIENT_IP}" ]]; then
    break
  fi
  sleep 2
done

if [[ -z "${BUILDER_IP}" || -z "${CLIENT_IP}" ]]; then
  log "Failed to determine VM IPs"
  exit 1
fi

log "Builder IP: ${BUILDER_IP}"
log "Client IP: ${CLIENT_IP}"

REPO_TAR="${WORKDIR}/repo-under-test.tar"
git -C "${WORKDIR}" archive --format=tar HEAD > "${REPO_TAR}"

log "Pushing repository snapshot into builder VM"
${INCUS_BIN} file push "${REPO_TAR}" "${BUILDER}/root/repo-under-test.tar"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'mkdir -p /root/nixos-builder-mon && tar -xf /root/repo-under-test.tar -C /root/nixos-builder-mon'

log "Ensuring nix-daemon is active"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'systemctl start nix-daemon'

log "Verifying external connectivity from builder"
log "Testing ping to gateway from builder"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "ping -c 1 -W 2 10.74.46.1" 2>&1 || log "Ping to gateway failed"
log "Testing ping to DNS from builder"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "ping -c 1 -W 2 8.8.8.8" 2>&1 || log "Ping to DNS failed"
log "Testing DNS resolution for cache.nixos.org"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "getent hosts cache.nixos.org || echo 'DNS resolution failed'" || log "DNS resolution test failed"
log "Testing DNS resolution for index.crates.io"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "getent hosts index.crates.io || echo 'DNS resolution failed'" || log "DNS resolution test failed"
log "Testing HTTP access to index.crates.io"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'curl -s --max-time 10 https://index.crates.io/config.json | head -c 100' 2>&1 || log "HTTP access to crates.io failed"
log "Configuring cargo to prefer IPv4"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "mkdir -p /root/.cargo && cat > /root/.cargo/config.toml <<'EOF'
[net]
git-fetch-with-cli = true
EOF
" || true
for _ in $(seq 1 5); do
  if ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'curl -s --max-time 5 https://cache.nixos.org/nix-cache-info >/dev/null 2>&1'; then
    break
  fi
  log "Retrying external connectivity check..."
  sleep 2
done

log "Waiting for nix-daemon to be ready"
wait_for_nix_daemon() {
  local inst="$1"
  local attempts=30
  for _ in $(seq 1 ${attempts}); do
    if ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc "test -S /nix/var/nix/daemon-socket/socket && /run/current-system/sw/bin/nix --version 2>/dev/null"; then
      return 0
    fi
    log "Waiting for nix-daemon on ${inst}..."
    sleep 2
  done
  log "nix-daemon did not become ready for ${inst}"
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'systemctl status nix-daemon || true'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'ls -la /nix/var/nix/daemon-socket/ 2>/dev/null || true'
  exit 1
}

wait_for_nix_daemon "${BUILDER}"

log "Building with dx (fullstack: server binary + web assets in correct layout)"
# nix run .#dx-build runs: dx build --platform web --release --fullstack true --features web
# Dioxus 0.7 CLI outputs to target/dx/{name}/release/web/ with the server binary
# at the root and public/ alongside it - no DIOXUS_ASSET_ROOT gymnastics needed.
timeout 1800 ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc \
  "cd /root/nixos-builder-mon && /run/current-system/sw/bin/nix run ${NIX_FLAGS} --option substituters https://cache.nixos.org .#dx-build"

# Dioxus 0.7 CLI outputs to target/dx/{name}/{profile}/web/ (not dist/)
DIST_DIR="/root/nixos-builder-mon/target/dx/nixos-buildermon/release/web"
log "dx build output:"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "find ${DIST_DIR} -maxdepth 3 | sort"

# NixOS systemd services run with a minimal PATH; pass the full profile PATH.
NIXOS_PATH="/run/current-system/sw/bin:/run/current-system/sw/sbin:/usr/local/bin:/usr/bin:/bin"

log "Starting daemon log forwarder via systemd-run"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc \
  "systemd-run --unit=nom-forwarder --description='NOM log forwarder' \
    --setenv=PATH=${NIXOS_PATH} \
    /bin/sh -c 'touch /var/log/nom-output.log; journalctl -u nix-daemon -n 0 --no-pager --no-hostname -o cat -f 2>&1 | tee -a /var/log/nom-output.log'"
log "Daemon log forwarder started (returned from incus exec - good)"

log "Starting nixos-buildermon web server via systemd-run (CWD=dist so it finds public/)"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc \
  "systemd-run --unit=nixos-buildermon --description='nixos-buildermon server' \
    --setenv=PATH=${NIXOS_PATH} \
    --setenv=IP=0.0.0.0 \
    --setenv=PORT=${MONITOR_PORT} \
    --working-directory=${DIST_DIR} \
    ${DIST_DIR}/nixos-buildermon"
log "Web server started (returned from incus exec - good)"

# NixOS firewall blocks all incoming ports by default; open the monitor port.
log "Opening port ${MONITOR_PORT} in NixOS firewall"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc \
  "nft add rule inet nixos-fw input-allow tcp dport ${MONITOR_PORT} accept 2>/dev/null || \
   iptables -I INPUT 1 -p tcp --dport ${MONITOR_PORT} -j ACCEPT 2>/dev/null || true"

# Brief pause then check unit status so a crash is visible in logs immediately
sleep 2
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'systemctl status nixos-buildermon --no-pager 2>&1 || true'

log "Waiting for web UI health endpoint"
for _ in $(seq 1 30); do
  if curl --fail --silent --max-time 5 "http://${BUILDER_IP}:${MONITOR_PORT}/" >/dev/null; then
    break
  fi
  sleep 2
done
curl --fail --silent --max-time 10 "http://${BUILDER_IP}:${MONITOR_PORT}/" >/dev/null

log "Preparing SSH access from client to builder"
timeout 60 ${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'if systemctl list-unit-files | grep -q "^sshd.service"; then systemctl enable --now sshd; elif systemctl list-unit-files | grep -q "^ssh.service"; then systemctl enable --now ssh; fi'

${INCUS_BIN} exec "${CLIENT}" -- /bin/sh -lc 'mkdir -p /root/.ssh && chmod 700 /root/.ssh && if [ ! -f /root/.ssh/id_ed25519 ]; then ssh-keygen -q -t ed25519 -N "" -f /root/.ssh/id_ed25519; fi'
${INCUS_BIN} file pull "${CLIENT}/root/.ssh/id_ed25519.pub" "${WORKDIR}/client_id_ed25519.pub"
${INCUS_BIN} file push "${WORKDIR}/client_id_ed25519.pub" "${BUILDER}/root/client_id_ed25519.pub"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'mkdir -p /root/.ssh && chmod 700 /root/.ssh && cat /root/client_id_ed25519.pub >> /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys'

log "Writing marker flake on client"
# Use builtins.derivation with no inputs so we never fetch nixpkgs from
# GitHub (that tarball is huge and reliably hangs inside the VM).
# Nix provides /bin/sh in the build sandbox automatically.
cat > "${WORKDIR}/remote-test-flake.nix" <<EOF
{
  outputs = { self }: {
    packages.x86_64-linux.marker = builtins.derivation {
      name = "incus-marker";
      system = "x86_64-linux";
      builder = "/bin/sh";
      args = [ "-c" "echo '${MARKER}' >&2; mkdir \$out; echo ok > \$out/result" ];
    };
  };
}
EOF

${INCUS_BIN} file push --create-dirs "${WORKDIR}/remote-test-flake.nix" "${CLIENT}/root/remote-test/flake.nix"

log "Triggering remote build from client -> builder"
timeout 300 ${INCUS_BIN} exec "${CLIENT}" -- /bin/sh -lc "export NIX_SSHOPTS='-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=30 -o ServerAliveInterval=10 -o ServerAliveCountMax=3'; /run/current-system/sw/bin/nix build ${NIX_FLAGS} --option substituters https://cache.nixos.org /root/remote-test#marker --max-jobs 0 --builders 'ssh-ng://root@${BUILDER_IP} x86_64-linux' -L"

log "Waiting for forwarder to flush marker"
sleep 6

${INCUS_BIN} file pull "${BUILDER}/var/log/nom-output.log" "${ARTIFACT_DIR}/builder-nom-output.log"

if ! grep -q "${MARKER}" "${ARTIFACT_DIR}/builder-nom-output.log"; then
  log "Marker not found in /var/log/nom-output.log"
  exit 1
fi

log "Smoke test passed: marker found in monitor log stream"

if [[ -n "${GITHUB_OUTPUT:-}" ]]; then
  {
    printf 'builder_ip=%s\n' "${BUILDER_IP}"
    printf 'monitor_port=%s\n' "${MONITOR_PORT}"
    printf 'marker=%s\n' "${MARKER}"
  } >> "${GITHUB_OUTPUT}"
fi

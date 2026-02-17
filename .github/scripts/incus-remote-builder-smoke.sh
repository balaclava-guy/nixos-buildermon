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
  printf '[incus-smoke] %s\n' "$*"
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

trap cleanup EXIT

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
    "images:nixos/unstable/default" \
    "images:nixos/unstable" \
    "images:nixos/25.11/default" \
    "images:nixos/25.11"; do
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

# Use VMs only when the image variant is cloud; otherwise launch containers
LAUNCH_OPTS=()
if [[ "${IMAGE}" == *"/cloud"* ]]; then
  LAUNCH_OPTS+=(--vm)
fi

log "Launching builder VM"
${INCUS_BIN} launch "${IMAGE}" "${BUILDER}" "${LAUNCH_OPTS[@]}" -c limits.cpu=4 -c limits.memory=8GiB

log "Launching client VM"
${INCUS_BIN} launch "${IMAGE}" "${CLIENT}" "${LAUNCH_OPTS[@]}" -c limits.cpu=2 -c limits.memory=4GiB

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

NIX_FLAGS="--extra-experimental-features flakes --extra-experimental-features nix-command --accept-flake-config --option flake-registry ''"

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

log "Waiting for nix-daemon to be ready"
wait_for_nix_daemon() {
  local inst="$1"
  local attempts=30
  for _ in $(seq 1 ${attempts}); do
    if ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc "test -S /nix/var/nix/daemon-socket/socket && /nix/var/nix/profiles/default/bin/nix --version 2>/dev/null"; then
      return 0
    fi
    log "Waiting for nix-daemon on ${inst}..."
    sleep 2
  done
  log "nix-daemon did not become ready for ${inst}"
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'systemctl status nix-daemon || true'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'ls -la /nix/var/nix/daemon-socket/ 2>/dev/null || true'
  ${INCUS_BIN} exec "${inst}" -- /bin/sh -lc 'which nix || echo "nix not found"; ls -la /nix/var/nix/profiles/default/bin/ | head -20 || true' 2>/dev/null || true
  exit 1
}

wait_for_nix_daemon "${BUILDER}"

log "Building nix-output-monitor binary path"
NOM_OUT="$(${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "/nix/var/nix/profiles/default/bin/nix build ${NIX_FLAGS} --no-link --print-out-paths nixpkgs#nix-output-monitor" | tr -d '\r')"
NOM_BIN="${NOM_OUT}/bin/nom"

log "Building server package"
SERVER_OUT="$(${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "cd /root/nixos-builder-mon && /nix/var/nix/profiles/default/bin/nix build ${NIX_FLAGS} --no-link --print-out-paths .#server" | tr -d '\r')"

log "Building web assets package"
WEB_OUT="$(${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "cd /root/nixos-builder-mon && /nix/var/nix/profiles/default/bin/nix build ${NIX_FLAGS} --no-link --print-out-paths .#web" | tr -d '\r')"

log "Starting daemon log forwarder (journalctl -> nom -> /var/log/nom-output.log)"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "touch /var/log/nom-output.log && nohup /bin/sh -lc 'exec journalctl -u nix-daemon -n 0 --no-pager --no-hostname -o cat -f 2>&1 | ${NOM_BIN} | tee -a /var/log/nom-output.log' >/var/log/nom-forwarder.log 2>&1 &"

log "Starting nixos-builder-mon web server"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc "nohup /bin/sh -lc 'exec env DIOXUS_ASSET_ROOT=${WEB_OUT} IP=0.0.0.0 PORT=${MONITOR_PORT} ${SERVER_OUT}/bin/nixos-builder-mon' >/var/log/nixos-builder-mon.log 2>&1 &"

log "Waiting for web UI health endpoint"
for _ in $(seq 1 30); do
  if curl --fail --silent "http://${BUILDER_IP}:${MONITOR_PORT}/" >/dev/null; then
    break
  fi
  sleep 2
done
curl --fail --silent "http://${BUILDER_IP}:${MONITOR_PORT}/" >/dev/null

log "Preparing SSH access from client to builder"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'if systemctl list-unit-files | grep -q "^sshd.service"; then systemctl enable --now sshd; elif systemctl list-unit-files | grep -q "^ssh.service"; then systemctl enable --now ssh; fi'

${INCUS_BIN} exec "${CLIENT}" -- /bin/sh -lc 'mkdir -p /root/.ssh && chmod 700 /root/.ssh && if [ ! -f /root/.ssh/id_ed25519 ]; then ssh-keygen -q -t ed25519 -N "" -f /root/.ssh/id_ed25519; fi'
${INCUS_BIN} file pull "${CLIENT}/root/.ssh/id_ed25519.pub" "${WORKDIR}/client_id_ed25519.pub"
${INCUS_BIN} file push "${WORKDIR}/client_id_ed25519.pub" "${BUILDER}/root/client_id_ed25519.pub"
${INCUS_BIN} exec "${BUILDER}" -- /bin/sh -lc 'mkdir -p /root/.ssh && chmod 700 /root/.ssh && cat /root/client_id_ed25519.pub >> /root/.ssh/authorized_keys && chmod 600 /root/.ssh/authorized_keys'

log "Writing marker flake on client"
cat > "${WORKDIR}/remote-test-flake.nix" <<EOF
{
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      pkgs = nixpkgs.legacyPackages.x86_64-linux;
    in {
      packages.x86_64-linux.marker = pkgs.runCommand "incus-marker" {} ''
        echo "${MARKER}" >&2
        mkdir -p \$out
        echo ok > \$out/result
      '';
    };
}
EOF

${INCUS_BIN} file push --create-dirs "${WORKDIR}/remote-test-flake.nix" "${CLIENT}/root/remote-test/flake.nix"

log "Triggering remote build from client -> builder"
${INCUS_BIN} exec "${CLIENT}" -- /bin/sh -lc "export NIX_SSHOPTS='-o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null'; /nix/var/nix/profiles/default/bin/nix build ${NIX_FLAGS} /root/remote-test#marker --max-jobs 0 --builders 'ssh-ng://root@${BUILDER_IP} x86_64-linux' -L"

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

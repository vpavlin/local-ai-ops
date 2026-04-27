#!/usr/bin/env bash
# Install laio binaries, systemd user units, and example config.
# Run from the agent-scheduler directory.
set -euo pipefail

BIN_DIR="${HOME}/.local/bin"
SYSTEMD_DIR="${HOME}/.config/systemd/user"
CONFIG_DIR="${HOME}/.config/laio"

# Build release binaries
echo "Building release binaries..."
cargo build --release

# Install binaries
mkdir -p "${BIN_DIR}"
for bin in laio-orchestrator laio-dispatcher laio-admin; do
    install -m 755 "target/release/${bin}" "${BIN_DIR}/${bin}"
    echo "  installed ${BIN_DIR}/${bin}"
done

# Install systemd units
mkdir -p "${SYSTEMD_DIR}"
for unit in systemd/*.service systemd/*.timer; do
    install -m 644 "${unit}" "${SYSTEMD_DIR}/$(basename "${unit}")"
    echo "  installed ${SYSTEMD_DIR}/$(basename "${unit}")"
done

# Install example config if no config exists yet
mkdir -p "${CONFIG_DIR}"
if [[ ! -f "${CONFIG_DIR}/config.yaml" ]]; then
    install -m 600 config.yaml.example "${CONFIG_DIR}/config.yaml"
    echo "  installed example config at ${CONFIG_DIR}/config.yaml — edit it before starting"
else
    echo "  config.yaml already exists, skipping"
fi

# Reload systemd and enable timers
systemctl --user daemon-reload
systemctl --user enable --now laio-orchestrator.timer laio-dispatcher.timer

echo ""
echo "Done. Next steps:"
echo "  1. Edit ${CONFIG_DIR}/config.yaml"
echo "  2. Set GH_TOKEN in ~/.config/environment.d/laio.conf"
echo "  3. Start the lemonade-idle sampler (see local-ai-ops/lemonade-idle/)"
echo "  4. Build the container image: podman build -t local-ai-ops-runner:latest container/"
echo ""
echo "Check status:"
echo "  systemctl --user status laio-orchestrator.timer laio-dispatcher.timer"
echo "  laio-admin tasks list"

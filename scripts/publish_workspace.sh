#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# WirePlant Workspace Topological Release Script
# ==============================================================================
# Publishes all 9 workspace crates to crates.io in topological dependency order.
# Usage:
#   ./scripts/publish_workspace.sh --dry-run
#   ./scripts/publish_workspace.sh --publish
# ==============================================================================

MODE="${1:-}"

if [[ "$MODE" != "--dry-run" && "$MODE" != "--publish" ]]; then
    echo "Usage: $0 [--dry-run | --publish]"
    exit 1
fi

FLAGS=""
if [[ "$MODE" == "--dry-run" ]]; then
    echo "=== Running in DRY-RUN mode (packages validated locally, nothing uploaded) ==="
    FLAGS="--dry-run --allow-dirty"
else
    echo "=== RUNNING IN LIVE PUBLISH MODE ==="
    read -p "Are you sure you want to publish WirePlant v0.1.0 to crates.io? (y/N) " -n 1 -r
    echo
    if [[ ! $REPLY =~ ^[Yy]$ ]]; then
        echo "Aborted."
        exit 1
    fi
    FLAGS=""
fi

# Topological publishing order (independent base crates first, consumers last)
CRATES=(
    "wireplant_core"
    "wireplant_engine"
    "wireplant_config"
    "wireplant_modbus"
    "wireplant_opcua"
    "wireplant_mqtt"
    "wireplant_api"
    "wireplant_tui"
    "wireplant"
)

INDEX_DELAY=20

for CRATE in "${CRATES[@]}"; do
    echo "------------------------------------------------------------"
    echo "Processing: $CRATE ($MODE)"
    echo "------------------------------------------------------------"

    if [[ "$MODE" == "--dry-run" ]]; then
        echo "Validating package contents for $CRATE:"
        cargo package -p "$CRATE" --list --allow-dirty
    else
        cargo publish -p "$CRATE" $FLAGS
        if [[ "$CRATE" != "${CRATES[-1]}" ]]; then
            echo "Waiting ${INDEX_DELAY}s for crates.io index to update before dependent crates..."
            sleep "$INDEX_DELAY"
        fi
    fi
done

echo "============================================================"
echo "All workspace crates processed successfully in topological order!"
echo "============================================================"

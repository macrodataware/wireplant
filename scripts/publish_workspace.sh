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
START_FROM="${2:-}"

if [[ "$MODE" != "--dry-run" && "$MODE" != "--publish" ]]; then
    echo "Usage: $0 [--dry-run | --publish] [start_from_crate]"
    exit 1
fi

# Extract current workspace package version dynamically from root Cargo.toml
VERSION=$(cargo metadata --no-deps --format-version 1 | grep -o '"version":"[^"]*"' | head -n 1 | cut -d'"' -f4)

if [[ -z "$VERSION" ]]; then
    VERSION=$(grep -m 1 '^version = ' Cargo.toml | cut -d'"' -f2)
fi

FLAGS=""
if [[ "$MODE" == "--dry-run" ]]; then
    echo "=== Running in DRY-RUN mode for v${VERSION} (packages validated locally, nothing uploaded) ==="
    FLAGS="--dry-run --allow-dirty"
else
    echo "=== RUNNING IN LIVE PUBLISH MODE for v${VERSION} ==="
    read -p "Are you sure you want to publish WirePlant v${VERSION} to crates.io? (y/N) " -n 1 -r
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

SKIP=false
if [[ -n "$START_FROM" ]]; then
    SKIP=true
fi

for CRATE in "${CRATES[@]}"; do
    if [[ "$SKIP" == "true" ]]; then
        if [[ "$CRATE" == "$START_FROM" ]]; then
            SKIP=false
        else
            echo "Skipping already published crate: $CRATE"
            continue
        fi
    fi

    echo "------------------------------------------------------------"
    echo "Processing: $CRATE v${VERSION} ($MODE)"
    echo "------------------------------------------------------------"

    if [[ "$MODE" == "--dry-run" ]]; then
        echo "Validating package contents for $CRATE:"
        cargo package -p "$CRATE" --list --allow-dirty
    else
        while true; do
            set +e
            OUTPUT=$(cargo publish -p "$CRATE" $FLAGS 2>&1)
            STATUS=$?
            set -e
            echo "$OUTPUT"

            if [[ $STATUS -eq 0 ]]; then
                break
            fi

            # If crate version is already published on crates.io, skip cleanly
            if echo "$OUTPUT" | grep -qi "already uploaded"; then
                echo "Crate $CRATE v${VERSION} is already published on crates.io. Skipping."
                break
            fi

            # Crates.io rate limit (HTTP 429) backoff
            if echo "$OUTPUT" | grep -q "429 Too Many Requests"; then
                echo "============================================================"
                echo "Hit crates.io rate limit (status 429) on crate $CRATE."
                echo "Waiting 600s (10 minutes) for token bucket to refill..."
                echo "============================================================"
                sleep 600
                echo "Retrying publish for $CRATE..."
            else
                echo "Publish failed with error for $CRATE."
                exit $STATUS
            fi
        done

        if [[ "$CRATE" != "${CRATES[-1]}" ]]; then
            echo "Waiting ${INDEX_DELAY}s for crates.io index to update before dependent crates..."
            sleep "$INDEX_DELAY"
        fi
    fi
done

echo "============================================================"
echo "All workspace crates processed successfully in topological order!"
echo "============================================================"

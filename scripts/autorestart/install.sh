#!/usr/bin/env bash
#
# Install the auto-restart unit for an EvaporChain validator.
#
# Auto-detects platform (macOS launchd vs Linux systemd), substitutes
# the validator id + user/home placeholders into the appropriate
# template, installs it, and (on systemd) enables + starts it.
#
# Run on each validator host individually:
#   ssh validator-host
#   cd ~/EvaporChain
#   ./scripts/autorestart/install.sh <VALIDATOR_ID>
#
# To uninstall:
#   ./scripts/autorestart/install.sh <VALIDATOR_ID> --uninstall
#
# This script intentionally does NOT enable the unit by default on
# macOS (`launchctl load` is operator-explicit). On Linux it does
# (`systemctl enable --now`), matching the typical sysadmin
# expectation for a service unit.

set -euo pipefail

if [[ -z "${1:-}" ]]; then
    echo "usage: $0 <VALIDATOR_ID> [--uninstall]" >&2
    exit 1
fi

VID="$1"
UNINSTALL="${2:-}"

if [[ ! "$VID" =~ ^[0-9]+$ ]]; then
    echo "VALIDATOR_ID must be numeric (got: $VID)" >&2
    exit 1
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
HOMEDIR="$HOME"
USER_NAME="$(id -un)"

case "$(uname -s)" in
    Darwin)
        # macOS — install LaunchAgent
        TARGET="$HOMEDIR/Library/LaunchAgents/com.evaporchain.validator-$VID.plist"
        if [[ "$UNINSTALL" == "--uninstall" ]]; then
            launchctl unload "$TARGET" 2>/dev/null || true
            rm -f "$TARGET"
            echo "Uninstalled LaunchAgent for validator $VID."
            exit 0
        fi
        sed -e "s|{{VALIDATOR_ID}}|$VID|g" \
            -e "s|{{HOME}}|$HOMEDIR|g" \
            "$SCRIPT_DIR/com.evaporchain.validator.plist" > "$TARGET"
        chmod 644 "$TARGET"
        launchctl unload "$TARGET" 2>/dev/null || true
        launchctl load   "$TARGET"
        echo "Installed + loaded LaunchAgent for validator $VID at $TARGET."
        echo "Logs: tail -f $HOMEDIR/evaporchain-5node-v$VID.launchd.log"
        ;;
    Linux)
        # Linux — install systemd unit (requires root)
        TARGET="/etc/systemd/system/evaporchain-validator-$VID.service"
        if [[ "$UNINSTALL" == "--uninstall" ]]; then
            sudo systemctl disable --now "evaporchain-validator-$VID.service" 2>/dev/null || true
            sudo rm -f "$TARGET"
            sudo systemctl daemon-reload
            echo "Uninstalled systemd unit for validator $VID."
            exit 0
        fi
        sed -e "s|{{VALIDATOR_ID}}|$VID|g" \
            -e "s|{{HOME}}|$HOMEDIR|g" \
            -e "s|{{USER}}|$USER_NAME|g" \
            "$SCRIPT_DIR/evaporchain-validator.service" | sudo tee "$TARGET" > /dev/null
        sudo systemctl daemon-reload
        sudo systemctl enable --now "evaporchain-validator-$VID.service"
        echo "Installed + started systemd unit for validator $VID."
        echo "Logs: journalctl -u evaporchain-validator-$VID.service -f"
        ;;
    *)
        echo "Unsupported platform: $(uname -s)" >&2
        exit 1
        ;;
esac

#!/usr/bin/env bash
# Links the versioned in-repository vision unit into the current user's systemd manager.
# It does not start the service: JARVIS starts it only for an image request.
set -euo pipefail

repository_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
unit="$repository_root/packaging/systemd/jarvis-vision.service"

systemctl --user link "$unit"
systemctl --user daemon-reload
printf '%s\n' "JARVIS vision service registered. It will start on the first image request."

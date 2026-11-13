#!/bin/sh
# Installs Strata Network as a system service (systemd).
#   sudo ./install.sh [--bind 0.0.0.0:8080]
# Then open http://<this machine>:8080/ ; on the server itself the first visit creates the admin account.
set -eu
if [ "$(id -u)" -ne 0 ]; then
  echo "Run as root: sudo $0 $*" >&2
  exit 1
fi
here=$(cd "$(dirname "$0")" && pwd)
dest=/opt/strata-network
mkdir -p "$dest"
if systemctl is-active --quiet strata-network 2>/dev/null; then
  systemctl stop strata-network
fi
install -m 755 "$here/strata-network" "$dest/strata-network"
ln -sf "$dest/strata-network" /usr/local/bin/strata-network
"$dest/strata-network" service install "$@"
echo "Strata Network is installed. Without a browser on this machine, create the admin with: sudo strata-network user add admin"

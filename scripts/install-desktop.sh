#!/usr/bin/env bash
# Install financi's desktop entry and icons for the current Linux user
# (3.63). Wayland compositors ignore per-window icons and match the
# window's app_id ("financi") to a desktop entry instead, so the entry and
# the themed PNGs have to be installed for the taskbar to show the icon.
# Copies packaging/linux/financi.desktop to ~/.local/share/applications/
# and each committed PNG to ~/.local/share/icons/hicolor/<n>x<n>/apps/.
set -euo pipefail
cd "$(dirname "$0")/.."

apps=${HOME}/.local/share/applications
icons=${HOME}/.local/share/icons/hicolor
mkdir -p "$apps"
install -m 644 packaging/linux/financi.desktop "$apps/financi.desktop"
for n in 16 32 64 128 256 512 1024; do
    mkdir -p "$icons/${n}x${n}/apps"
    install -m 644 "assets/icon/png/financi-$n.png" "$icons/${n}x${n}/apps/financi.png"
done
# Refresh the caches when the tools are present; desktop environments
# pick the files up either way, eventually.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$apps" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 &&
    gtk-update-icon-cache -q "$icons" || true
echo "installed $apps/financi.desktop and $icons/<n>x<n>/apps/financi.png"

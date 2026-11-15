#!/bin/sh
# Installs L4D2 Addons Manager for the current user (no root needed; works on SteamOS).
#   ./install.sh            install into ~/.local
#   ./install.sh --uninstall
set -eu
cd "$(dirname "$0")"
PREFIX="${PREFIX:-$HOME/.local}"
BIN="$PREFIX/bin/l4d2-addons-manager"
DESKTOP="$PREFIX/share/applications/l4d2-addons-manager.desktop"
ICON="$PREFIX/share/icons/hicolor/256x256/apps/l4d2-addons-manager.png"

if [ "${1:-}" = "--uninstall" ]; then
    rm -f "$BIN" "$DESKTOP" "$ICON"
    echo "Uninstalled. Settings are kept in ${XDG_CONFIG_HOME:-$HOME/.config}/L4D2AddonManager"
    exit 0
fi

install -Dm755 l4d2-addons-manager "$BIN"
install -Dm644 l4d2-addons-manager.png "$ICON"
mkdir -p "$(dirname "$DESKTOP")"
# absolute Exec so the launcher works even when ~/.local/bin isn't on PATH
sed "s|^Exec=.*|Exec=$BIN|" l4d2-addons-manager.desktop > "$DESKTOP"
chmod 644 "$DESKTOP"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" 2>/dev/null || true
echo "Installed to $BIN. Look for \"L4D2 Addons Manager\" in your app menu."

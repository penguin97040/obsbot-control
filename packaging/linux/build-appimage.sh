#!/usr/bin/env bash
# Builds dist/obsbot-control-x86_64.AppImage from target/release/obsbot-control.
#
# Run `cargo build --release` first. The AppImage carries the app, its icon and
# desktop entry, and the keyboard libraries GPUI needs that a desktop might not have;
# graphics drivers, Vulkan, fonts and glibc always come from the system.
#
# APPIMAGETOOL may point at an appimagetool binary; otherwise it is downloaded.
set -euo pipefail

cd "$(dirname "$0")/../.."
binary=target/release/obsbot-control
[ -x "$binary" ] || { echo "Build the app first: cargo build --release" >&2; exit 1; }

appdir=AppDir
rm -rf "$appdir"
mkdir -p "$appdir/usr/bin" "$appdir/usr/lib" "$appdir/usr/share/applications" \
  "$appdir/usr/share/icons/hicolor/256x256/apps" "$appdir/usr/share/metainfo"

install -m 755 "$binary" "$appdir/usr/bin/obsbot-control"
install -m 644 packaging/linux/obsbot-control.desktop "$appdir/usr/share/applications/"
install -m 644 res/obsbot-control.png "$appdir/usr/share/icons/hicolor/256x256/apps/"
install -m 644 LICENSE "$appdir/usr/share/metainfo/obsbot-control.LICENSE"
ln -s usr/share/applications/obsbot-control.desktop "$appdir/obsbot-control.desktop"
ln -s usr/share/icons/hicolor/256x256/apps/obsbot-control.png "$appdir/obsbot-control.png"
ln -s obsbot-control.png "$appdir/.DirIcon"

# GPUI links these keyboard libraries; most desktops have them, but not all.
for lib in libxkbcommon.so.0 libxkbcommon-x11.so.0; do
  path=$(ldconfig -p | awk -v l="$lib" '$1 == l && /x86-64/ { print $NF; exit }')
  [ -n "$path" ] || { echo "Missing $lib on the build machine" >&2; exit 1; }
  cp -L "$path" "$appdir/usr/lib/$lib"
done

cat > "$appdir/AppRun" <<'EOF'
#!/bin/sh
# The bundled keyboard libraries; everything else comes from the system.
here="$(dirname "$(readlink -f "$0")")"
export LD_LIBRARY_PATH="${LD_LIBRARY_PATH:+$LD_LIBRARY_PATH:}$here/usr/lib"
exec "$here/usr/bin/obsbot-control" "$@"
EOF
chmod 755 "$appdir/AppRun"

tool="${APPIMAGETOOL:-}"
if [ -z "$tool" ]; then
  tool=$(mktemp -d)/appimagetool
  # appimagetool publishes only a rolling "continuous" build, so record which one was used.
  curl -fsSL -o "$tool" https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod 755 "$tool"
  echo "appimagetool SHA-256: $(sha256sum "$tool" | cut -d' ' -f1)"
fi

mkdir -p dist
# --appimage-extract-and-run lets appimagetool work without FUSE (as on CI runners).
ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream "$appdir" dist/obsbot-control-x86_64.AppImage
echo "Built dist/obsbot-control-x86_64.AppImage"

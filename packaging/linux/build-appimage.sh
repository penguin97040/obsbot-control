#!/usr/bin/env bash
# Builds dist/obsbot-control-x86_64.AppImage from target/release/obsbot-control.
#
# Run `cargo build --release` first. The AppImage carries the app, its icon and
# desktop entry, and the keyboard libraries GPUI needs that a desktop might not have;
# graphics drivers, Vulkan, fonts and glibc always come from the system.
#
# APPIMAGETOOL may point at an appimagetool binary; otherwise a pinned one is downloaded.
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

# appimagetool and the AppImage runtime are only published as rolling "continuous"
# builds, so they are pinned by checksum instead of by version. When upstream
# replaces them this fails; check the new files, then update both hashes here.
tool_url=https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-x86_64.AppImage
tool_sha256=e8c1169c4af66c3184d7ee110713d4be04befe671fa358e2c1591ee840553b03
runtime_url=https://github.com/AppImage/type2-runtime/releases/download/continuous/runtime-x86_64
runtime_sha256=617aa77314d57c8b8e4fae9c3f0b35af7b30d5529cb69e98f64f92510ed98a46

download() { # <url> <sha256> <file>
  curl -fsSL -o "$3" "$1"
  echo "$2  $3" | sha256sum -c --quiet - || {
    echo "Checksum mismatch for $1 (got $(sha256sum "$3" | cut -d' ' -f1))." >&2
    echo "Upstream has published a new build: check it, then update the pinned hash in $0." >&2
    exit 1
  }
}

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
tool="${APPIMAGETOOL:-}"
if [ -z "$tool" ]; then
  tool="$work/appimagetool"
  download "$tool_url" "$tool_sha256" "$tool"
  chmod 755 "$tool"
fi
download "$runtime_url" "$runtime_sha256" "$work/runtime"

mkdir -p dist
# --appimage-extract-and-run lets appimagetool work without FUSE (as on CI runners).
ARCH=x86_64 "$tool" --appimage-extract-and-run --no-appstream --runtime-file "$work/runtime" \
  "$appdir" dist/obsbot-control-x86_64.AppImage
echo "Built dist/obsbot-control-x86_64.AppImage"

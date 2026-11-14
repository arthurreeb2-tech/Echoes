#!/bin/sh
# Construit Echoes-macOS.pkg (Intel + Apple Silicon) à partir des deux binaires compilés.
#   packaging/macos/build-pkg.sh <version> <binaire-arm64> <binaire-x86_64> <sortie.pkg>
set -eu
version="$1"; arm="$2"; intel="$3"; out="$4"
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
app="$work/root/Applications/Echoes.app"
mkdir -p "$app/Contents/MacOS" "$work/scripts"
lipo -create "$arm" "$intel" -output "$app/Contents/MacOS/Echoes"
chmod +x "$app/Contents/MacOS/Echoes"
sed "s/__VERSION__/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
codesign --force --sign - "$app"
cp "$here/postinstall" "$work/scripts/postinstall"
pkgbuild --root "$work/root" --scripts "$work/scripts" --identifier io.echoes.pkg --version "$version" --install-location / "$out"
rm -rf "$work"

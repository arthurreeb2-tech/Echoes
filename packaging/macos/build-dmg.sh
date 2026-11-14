#!/bin/sh
# Construit Echoes-macOS.dmg (Intel + Apple Silicon) : on glisse Echoes dans Applications,
# et au premier lancement l'app installe le service puis ouvre le tableau de bord.
#   packaging/macos/build-dmg.sh <version> <binaire-arm64> <binaire-x86_64> <sortie.dmg>
set -eu
version="$1"; arm="$2"; intel="$3"; out="$4"
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
app="$work/Echoes/Echoes.app"
mkdir -p "$app/Contents/MacOS"
lipo -create "$arm" "$intel" -output "$app/Contents/MacOS/Echoes"
chmod +x "$app/Contents/MacOS/Echoes"
sed "s/__VERSION__/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
codesign --force --sign - "$app"
ln -s /Applications "$work/Echoes/Applications"
hdiutil create -volname "Echoes" -srcfolder "$work/Echoes" -ov -format UDZO "$out"
rm -rf "$work"

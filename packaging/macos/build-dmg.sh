#!/bin/sh
# Construit Echoes-macOS.dmg (Intel + Apple Silicon) : on glisse Echoes dans Applications,
# et au premier lancement l'app installe le service puis ouvre le tableau de bord.
#   packaging/macos/build-dmg.sh <version> <binaire-arm64> <binaire-x86_64> <sortie.dmg>
set -eu
version="$1"; arm="$2"; intel="$3"; out="$4"
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d)
app="$work/Echoes/Echoes.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
lipo -create "$arm" "$intel" -output "$app/Contents/MacOS/Echoes"
chmod +x "$app/Contents/MacOS/Echoes"
sed "s/__VERSION__/$version/g" "$here/Info.plist" > "$app/Contents/Info.plist"
# Icône de l'app (Intel et Apple Silicon : la même), rendue depuis brand/ par brand/rendre.js.
cp "$here/Echoes.icns" "$app/Contents/Resources/Echoes.icns"
codesign --force --sign - "$app"
ln -s /Applications "$work/Echoes/Applications"
# La même icône pour le volume monté, quand l'outil SetFile (Xcode) est là. Facultatif : un échec
# ici ne doit jamais bloquer une publication.
if command -v SetFile >/dev/null 2>&1; then
  { cp "$here/Echoes.icns" "$work/Echoes/.VolumeIcon.icns" && SetFile -c icnC "$work/Echoes/.VolumeIcon.icns" && SetFile -a C "$work/Echoes"; } \
    || { echo "icône du volume non appliquée" >&2; rm -f "$work/Echoes/.VolumeIcon.icns"; }
fi
hdiutil create -volname "Echoes" -srcfolder "$work/Echoes" -ov -format UDZO "$out"
rm -rf "$work"

#!/bin/sh
# Installe Echoes sur macOS ou Linux.
#
#   Première machine :   curl -fsSL https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.sh | sh
#   Machines suivantes : curl -fsSL https://raw.githubusercontent.com/arthurreeb2-tech/Echoes/main/install.sh | sh -s -- <code>
#
# Télécharge la dernière version publiée ; si aucune n'est disponible et que ce script
# est lancé depuis le dépôt avec Rust installé, compile depuis les sources.
set -eu

# Les versions sont publiées sur Echoes-Maj ; Echoes reste en secours.
REPOS="arthurreeb2-tech/Echoes-Maj arthurreeb2-tech/Echoes"
CODE="${1:-}"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64)                t=aarch64-apple-darwin ;;
  Darwin-x86_64)               t=x86_64-apple-darwin ;;
  Linux-x86_64)                t=x86_64-unknown-linux-gnu ;;
  Linux-aarch64|Linux-arm64)   t=aarch64-unknown-linux-gnu ;;
  *) echo "Echoes : système non pris en charge ($(uname -s) $(uname -m))" >&2; exit 1 ;;
esac

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
bin="$tmp/echoes"

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

src=""
if [ -f "./Cargo.toml" ] && grep -q '^name = "echoes"' ./Cargo.toml; then src="$(pwd)"; fi

if [ -n "$src" ] && [ "${ECHOES_FROM_SOURCE:-}" = "1" ]; then
  downloaded=no
else
  downloaded=no
  for repo in $REPOS; do
    url="https://github.com/$repo/releases/latest/download/echoes-$t"
    if curl -fsSL --proto '=https' "$url" -o "$bin" 2>/dev/null && curl -fsSL --proto '=https' "$url.sha256" -o "$bin.sha256" 2>/dev/null; then
      if [ "$(sha256 "$bin")" != "$(cut -d' ' -f1 < "$bin.sha256")" ]; then
        echo "Echoes : empreinte incorrecte, téléchargement refusé." >&2; exit 1
      fi
      downloaded=yes
      break
    fi
  done
fi

if [ "$downloaded" = no ]; then
  if [ -z "$src" ]; then
    echo "Echoes : aucune version publiée trouvée (Echoes-Maj ni Echoes)." >&2
    echo "Lance ce script depuis le dépôt cloné, avec Rust installé, pour compiler depuis les sources." >&2
    exit 1
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    [ -x "$HOME/.cargo/bin/cargo" ] && PATH="$HOME/.cargo/bin:$PATH"
  fi
  if ! command -v cargo >/dev/null 2>&1; then
    echo "Echoes : Rust est nécessaire pour compiler (https://rustup.rs)." >&2; exit 1
  fi
  echo "Compilation d'Echoes depuis les sources…"
  cargo build --release --locked --manifest-path "$src/Cargo.toml" >&2
  cp "$src/target/release/echoes" "$bin"
fi

chmod +x "$bin"
# shellcheck disable=SC2086
"$bin" install $CODE

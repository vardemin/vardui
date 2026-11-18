#!/bin/sh
# Install the latest `vard` release into ~/.vard/bin (or $VARD_INSTALL_DIR).
#   curl -fsSL https://raw.githubusercontent.com/vardemin/vardui/master/install.sh | sh
set -eu

repo="vardemin/vardui"
dir="${VARD_INSTALL_DIR:-$HOME/.vard/bin}"

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  *) echo "no prebuilt vard for $(uname -s) $(uname -m); build from source: cargo install --git https://github.com/$repo vard-cli" >&2; exit 1 ;;
esac

url="https://github.com/$repo/releases/latest/download/vard-$target.tar.gz"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT
curl -fsSL "$url" -o "$tmp/vard.tar.gz"
curl -fsSL "$url.sha256" -o "$tmp/vard.tar.gz.sha256"
want="$(cut -d' ' -f1 "$tmp/vard.tar.gz.sha256")"
if command -v sha256sum >/dev/null; then got="$(sha256sum "$tmp/vard.tar.gz" | cut -d' ' -f1)"; else got="$(shasum -a 256 "$tmp/vard.tar.gz" | cut -d' ' -f1)"; fi
[ "$want" = "$got" ] || { echo "checksum mismatch for $url" >&2; exit 1; }
mkdir -p "$dir"
tar -xzf "$tmp/vard.tar.gz" -C "$dir"
chmod +x "$dir/vard"
echo "installed $("$dir/vard" --version) to $dir/vard"
case ":$PATH:" in *":$dir:"*) ;; *) echo "add it to PATH: export PATH=\"$dir:\$PATH\"" ;; esac

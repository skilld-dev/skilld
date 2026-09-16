#!/bin/sh
# Install the skilld CLI without Node.js.
#
#   curl -fsSL https://github.com/skilld-dev/skilld/releases/latest/download/install.sh | sh
#
# Environment:
#   SKILLD_INSTALL_DIR  Install directory. Default: $HOME/.skilld/bin
#   SKILLD_VERSION      Exact version to install, for example 3.1.0. Default: latest
#
# The installed CLI upgrades itself after it verifies each signed release.
set -eu

RELEASE_PUBLIC_KEY="__SKILLD_RELEASE_PUBLIC_KEY__"
REPOSITORY="https://github.com/skilld-dev/skilld"

fail() {
  printf 'skilld install: %s\n' "$1" >&2
  exit 1
}

download() {
  if command -v curl >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
  elif command -v wget >/dev/null 2>&1; then
    wget --https-only -q "$1" -O "$2"
  else
    fail "Install curl or wget, then run this script again."
  fi
}

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk '{print $1}'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk '{print $1}'
  else
    fail "Install sha256sum or shasum, then run this script again."
  fi
}

base64url_decode() {
  value=$(printf '%s' "$1" | tr '_-' '/+')
  case $((${#value} % 4)) in
    2) value="$value==" ;;
    3) value="$value=" ;;
  esac
  printf '%s' "$value" | openssl base64 -d -A
}

# Verifies the release manifest signature when OpenSSL supports Ed25519.
# Returns 2 when this system cannot check the signature.
verify_signature() {
  case "$RELEASE_PUBLIC_KEY" in __*) return 2 ;; esac
  command -v openssl >/dev/null 2>&1 || return 2
  # DER SubjectPublicKeyInfo for a raw Ed25519 key.
  printf '\060\052\060\005\006\003\053\145\160\003\041\000' >"$work/key.der"
  base64url_decode "$RELEASE_PUBLIC_KEY" >>"$work/key.der" || return 2
  openssl pkey -pubin -inform DER -in "$work/key.der" -out "$work/key.pem" >/dev/null 2>&1 || return 2
  printf 'skilld-release-v1\000' >"$work/message"
  openssl dgst -sha256 -binary "$work/skilld-release.txt" >>"$work/message" || return 2
  base64url_decode "$(tr -d '\n' <"$work/skilld-release.sig")" >"$work/signature" || return 1
  openssl pkeyutl -verify -pubin -inkey "$work/key.pem" -rawin \
    -in "$work/message" -sigfile "$work/signature" >/dev/null 2>&1 && return 0
  # Distinguish a bad signature from an OpenSSL without Ed25519 support.
  openssl pkeyutl -help 2>&1 | grep -q -- '-rawin' || return 2
  return 1
}

case "$(uname -s)" in
  Linux) os=linux ;;
  Darwin) os=darwin ;;
  *) fail "This script supports Linux and macOS. On Windows, run install.ps1." ;;
esac
case "$(uname -m)" in
  x86_64 | amd64) arch=x64 ;;
  arm64 | aarch64) arch=arm64 ;;
  *) fail "skilld has no build for $(uname -m)." ;;
esac
if [ "$os" = darwin ]; then
  asset="skilld-cli-darwin-$arch"
else
  libc=gnu
  if ls /lib/ld-musl-* >/dev/null 2>&1 || (ldd --version 2>&1 | grep -qi musl); then
    libc=musl
  fi
  asset="skilld-cli-linux-$arch-$libc"
fi

if [ -n "${SKILLD_VERSION:-}" ]; then
  release="$REPOSITORY/releases/download/v$SKILLD_VERSION"
else
  release="$REPOSITORY/releases/latest/download"
fi
install_dir="${SKILLD_INSTALL_DIR:-$HOME/.skilld/bin}"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT INT TERM

download "$release/skilld-release.txt" "$work/skilld-release.txt"
download "$release/skilld-release.sig" "$work/skilld-release.sig"
set +e
verify_signature
signature_status=$?
set -e
case $signature_status in
  0) ;;
  2) printf 'skilld install: OpenSSL cannot check Ed25519 signatures here. Using the HTTPS download and SHA-256 digest only.\n' >&2 ;;
  *) fail "The release manifest signature is invalid. Nothing was installed." ;;
esac

[ "$(sed -n 1p "$work/skilld-release.txt")" = "skilld-release-v1" ] || fail "The release manifest is invalid."
version=$(sed -n 2p "$work/skilld-release.txt" | sed -n 's/^version \([0-9A-Za-z.-]*\)$/\1/p')
[ -n "$version" ] || fail "The release manifest has no version."
expected=$(awk -v name="$asset" '$2 == name { print $1 }' "$work/skilld-release.txt")
[ -n "$expected" ] || fail "The release has no $asset build."

download "$release/$asset" "$work/skilld"
[ "$(sha256 "$work/skilld")" = "$expected" ] || fail "The downloaded binary does not match its digest. Nothing was installed."
chmod 755 "$work/skilld"
[ "$("$work/skilld" --version)" = "skilld $version" ] || fail "The downloaded binary reports the wrong version."

mkdir -p "$install_dir"
cp "$work/skilld" "$install_dir/.skilld-install-$$"
mv -f "$install_dir/.skilld-install-$$" "$install_dir/skilld"
printf '{"channel":"standalone"}\n' >"$install_dir/skilld-install.json"

printf 'Installed skilld %s to %s/skilld.\n' "$version" "$install_dir"
case ":$PATH:" in
  *":$install_dir:"*) ;;
  *) printf 'Add %s to your PATH, then run skilld --version.\n' "$install_dir" ;;
esac

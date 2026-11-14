#!/bin/sh
set -eu

REPO="magicdian/oh-my-useless"
VERSION="${USELESS_VERSION:-latest}"
VERSION="${VERSION#v}"

fail() {
    printf 'useless-install: %s\n' "$*" >&2
    exit 1
}

[ "$(id -u)" -eq 0 ] || fail "run as root, for example: curl ... | sudo sh"
[ "$(uname -s)" = "Linux" ] || fail "useless runs on Linux only"

case "$(uname -m)" in
    x86_64 | amd64) arch=x86_64 ;;
    aarch64 | arm64) arch=aarch64 ;;
    *) fail "unsupported architecture: $(uname -m)" ;;
esac

# The musl build is static and runs on both glibc and musl hosts.
target="${USELESS_TARGET:-${arch}-unknown-linux-musl}"
asset="useless-${target}.tar.gz"
sum="${asset%.tar.gz}.sha256"

if [ "$VERSION" = "latest" ]; then
    base="https://github.com/${REPO}/releases/latest/download"
else
    base="https://github.com/${REPO}/releases/download/${VERSION}"
fi

fetch() {
    if command -v curl > /dev/null 2>&1; then
        curl -fsSL "$1" -o "$2"
    elif command -v wget > /dev/null 2>&1; then
        wget -qO "$2" "$1"
    else
        fail "need curl or wget"
    fi
}

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

printf 'downloading %s\n' "$asset"
fetch "${base}/${asset}" "${tmp}/${asset}"
fetch "${base}/${sum}" "${tmp}/${sum}"

if command -v sha256sum > /dev/null 2>&1; then
    (cd "$tmp" && sha256sum -c "${sum}")
elif command -v shasum > /dev/null 2>&1; then
    (cd "$tmp" && shasum -a 256 -c "${sum}")
else
    fail "need sha256sum or shasum"
fi

tar -xzf "${tmp}/${asset}" -C "$tmp"
[ -f "${tmp}/useless" ] || fail "archive did not contain useless"
chmod +x "${tmp}/useless"

"${tmp}/useless" install

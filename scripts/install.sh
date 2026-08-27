#!/bin/sh
set -eu

NAME=macstrap
REPO=b00gizm/macstrap

die() {
	echo "$1" >&2
	exit 1
}

[ "$(uname -s)" = Darwin ] || die "This installer runs on macOS only."

arch=$(uname -m)
case "$arch" in
arm64) arch=arm64 ;;
x86_64) arch=amd64 ;;
*) die "unsupported architecture: $arch" ;;
esac

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT INT HUP

base="https://github.com/${REPO}/releases/latest/download"
tarball="${NAME}_darwin_${arch}.tar.gz"

curl -fsSL "$base/$tarball" -o "$tmpdir/$tarball"
curl -fsSL "$base/checksums.txt" -o "$tmpdir/checksums.txt"

(
	cd "$tmpdir"
	command -v shasum >/dev/null 2>&1 || die "shasum not found"
	grep " ${tarball}\$" checksums.txt | shasum -a 256 -c -
)

tar -xzf "$tmpdir/$tarball" -C "$tmpdir"
[ -f "$tmpdir/$NAME" ] || die "archive did not contain $NAME"

bindir="${HOME}/.local/bin"
mkdir -p "$bindir"
mv "$tmpdir/$NAME" "$bindir/$NAME"
chmod +x "$bindir/$NAME"

case ":$PATH:" in
*":$bindir:"*) ;;
*) echo "add $bindir to PATH so the next shell finds $NAME" ;;
esac

if [ "$#" -gt 0 ]; then
	exec "$bindir/$NAME" "$@"
fi

echo "installed to $bindir/$NAME"
echo "run macstrap to bootstrap"

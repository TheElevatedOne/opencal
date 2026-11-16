#!/bin/sh
# Build a release tarball: dist/opencal-<version>-linux-<arch>.tar.gz, laid out for /usr.
# Usage: packaging/make-release-tarball.sh <version> [arch]
# Run `make build` first.
set -eu
version=$1
arch=${2:-$(uname -m)}
root=$(cd "$(dirname "$0")/.." && pwd)
name="opencal-$version-linux-$arch"
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

make -C "$root" install PREFIX=/usr DESTDIR="$stage/install" >/dev/null
test -x "$stage/install/usr/bin/opencal"
mkdir -p "$stage/$name"
cp -a "$stage/install/usr/." "$stage/$name/"
cp "$root/README.md" "$root/CHANGELOG.md" "$root/LICENSE" "$stage/$name/"
mkdir -p "$root/dist"
tar --owner=0 --group=0 -C "$stage" -czf "$root/dist/$name.tar.gz" "$name"
(cd "$root/dist" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256")
echo "$root/dist/$name.tar.gz"

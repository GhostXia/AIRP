#!/usr/bin/env bash
# Build portable AIRP WebUI package for Linux (x86_64-unknown-linux-musl, static).
# Mirrors deploy/windows-webui/build.ps1.
set -euo pipefail

deploy_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$deploy_root/../.." && pwd)"
dist_root="$repo_root/dist"
package_root="$dist_root/airp-webui-linux-x64"
archive="$dist_root/airp-webui-linux-x64.tar.gz"

# Safety: refuse to stage outside dist.
case "$package_root" in
    "$dist_root"/*) ;;
    *) echo "Refusing to stage outside $dist_root" >&2; exit 1 ;;
esac

target_triple="x86_64-unknown-linux-musl"

# Ensure musl-gcc is discoverable when compiling bundled C deps (rusqlite, ring, etc.).
export CC_x86_64_unknown_linux_musl="${CC_x86_64_unknown_linux_musl:-musl-gcc}"
export CXX_x86_64_unknown_linux_musl="${CXX_x86_64_unknown_linux_musl:-musl-g++}"
export AR_x86_64_unknown_linux_musl="${AR_x86_64_unknown_linux_musl:-ar}"

cd "$repo_root"

if [[ "${1:-}" != "--skip-build" ]]; then
    cargo build -p airp-core --bin airp-core --release --locked --target "$target_triple"
fi

engine="$repo_root/target/$target_triple/release/airp-core"
if [[ ! -f "$engine" ]]; then
    echo "Missing release engine: $engine" >&2
    exit 1
fi

# Reject accidental dynamic linking so we never ship a glibc-dependent binary.
ldd_output="$(ldd "$engine" 2>&1 || true)"
if ! echo "$ldd_output" | grep -qE "not a dynamic executable|statically linked"; then
    echo "Engine is not a static executable:" >&2
    echo "$ldd_output" >&2
    exit 1
fi

if [[ -e "$package_root" ]]; then
    rm -rf "$package_root"
fi
mkdir -p "$package_root/webui"

cp "$engine" "$package_root/airp-core"
cp "$repo_root/webui/index.html" "$package_root/webui/"
cp -r "$repo_root/webui/assets" "$package_root/webui/"
cp -r "$repo_root/webui/screens" "$package_root/webui/"
cp "$deploy_root/start-airp.sh" "$package_root/"
cp "$deploy_root/README.txt" "$package_root/"
cp "$repo_root/LICENSE-MIT" "$package_root/"
cp "$repo_root/LICENSE-APACHE" "$package_root/"

chmod 0555 "$package_root/airp-core" "$package_root/start-airp.sh"

rm -f "$archive"
tar -C "$dist_root" -czf "$archive" "airp-webui-linux-x64"
echo "Created $archive"

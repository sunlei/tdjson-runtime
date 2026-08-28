#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 7 ]]; then
    printf 'usage: %s <install-dir> <source-dir> <version> <upstream-commit> <target> <compiler> <new-dist-dir>\n' "$0" >&2
    exit 2
fi

install_dir=$1
source_dir=$2
version=$3
upstream_commit=$4
target=$5
compiler=$6
dist_dir=$7
short_commit=${upstream_commit:0:12}
package_name="tdjson-v${version}-g${short_commit}-${target}"
package_dir="$dist_dir/$package_name"
archive="$dist_dir/$package_name.tar.zst"
metadata="$dist_dir/$target.metadata.json"

if [[ -e "$dist_dir" ]]; then
    printf 'distribution directory already exists: %s\n' "$dist_dir" >&2
    exit 2
fi

mkdir -p "$package_dir/lib"
cp "$source_dir/LICENSE_1_0.txt" "$package_dir/"

jq -n \
    --arg tdlib_version "$version" \
    --arg tdlib_commit "$upstream_commit" \
    --arg target "$target" \
    --arg compiler "$compiler" \
    '{format_version: 1, tdlib_version: $tdlib_version, tdlib_commit: $tdlib_commit, target: $target, compiler: $compiler}' \
    > "$package_dir/BUILD-METADATA.json"

case $(uname -s) in
    Darwin)
        if [[ ! -f "${OPENSSL_LICENSE_FILE:-}" ]]; then
            printf 'OPENSSL_LICENSE_FILE must name the OpenSSL license included by the macOS build\n' >&2
            exit 2
        fi
        cp "$OPENSSL_LICENSE_FILE" "$package_dir/OPENSSL-LICENSE.txt"
        cp -a "$install_dir"/lib/libtdjson*.dylib "$package_dir/lib/"
        library=$(find "$package_dir/lib" -type f -name 'libtdjson*.dylib' -print -quit)
        otool -L "$library" > "$package_dir/DYNAMIC_DEPENDENCIES.txt"
        ;;
    Linux)
        cp -a "$install_dir"/lib/libtdjson.so* "$package_dir/lib/"
        library=$(find "$package_dir/lib" -type f -name 'libtdjson.so.*' -print -quit)
        ldd "$library" > "$package_dir/DYNAMIC_DEPENDENCIES.txt"
        ;;
    *)
        printf 'unsupported packaging host: %s\n' "$(uname -s)" >&2
        exit 2
        ;;
esac

tar -C "$dist_dir" -cf - "$package_name" | zstd --threads=0 -19 -o "$archive"

case $(uname -s) in
    Darwin)
        sha256=$(shasum -a 256 "$archive" | awk '{print $1}')
        ;;
    Linux)
        sha256=$(sha256sum "$archive" | awk '{print $1}')
        ;;
esac

jq -n \
    --arg file "$(basename "$archive")" \
    --arg tdlib_version "$version" \
    --arg tdlib_commit "$upstream_commit" \
    --arg target "$target" \
    --arg compiler "$compiler" \
    --arg sha256 "$sha256" \
    '{file: $file, tdlib_version: $tdlib_version, tdlib_commit: $tdlib_commit, target: $target, compiler: $compiler, sha256: $sha256}' \
    > "$metadata"

printf '%s\n' "$archive"

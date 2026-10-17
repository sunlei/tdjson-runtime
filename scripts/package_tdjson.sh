#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 6 ]]; then
    printf 'usage: %s <install-dir> <source-dir> <version> <upstream-commit> <target> <new-dist-dir>\n' "$0" >&2
    exit 2
fi

install_dir=$1
source_dir=$2
version=$3
upstream_commit=$4
target=$5
dist_dir=$6
short_commit=${upstream_commit:0:12}
package_name="tdjson-v${version}-g${short_commit}-${target}"
package_dir="$dist_dir/$package_name"
archive="$dist_dir/$package_name.tar.zst"
metadata="$dist_dir/$target.metadata.json"
shopt -s nullglob

if [[ -e "$dist_dir" ]]; then
    printf 'distribution directory already exists: %s\n' "$dist_dir" >&2
    exit 2
fi

# Release builds use build_tdjson.sh's default compiler. Do not use $CXX here: it may hold a
# launcher command such as "sccache clang++".
compiler=$(clang++ --version)
compiler=${compiler%%$'\n'*}
if [[ -z "$compiler" ]]; then
    printf 'compiler reported an empty version string\n' >&2
    exit 2
fi

mkdir -p "$package_dir/lib"
cp "$source_dir/LICENSE_1_0.txt" "$package_dir/"

build_metadata=$(jq -n \
    --arg tdlib_version "$version" \
    --arg tdlib_commit "$upstream_commit" \
    --arg target "$target" \
    --arg compiler "$compiler" \
    '{format_version: 1, tdlib_version: $tdlib_version, tdlib_commit: $tdlib_commit, target: $target, compiler: $compiler}')
printf '%s\n' "$build_metadata" > "$package_dir/BUILD-METADATA.json"

case $(uname -s) in
    Darwin)
        if [[ ! -f "${OPENSSL_LICENSE_FILE:-}" ]]; then
            printf 'OPENSSL_LICENSE_FILE must name the OpenSSL license included by the macOS build\n' >&2
            exit 2
        fi
        cp "$OPENSSL_LICENSE_FILE" "$package_dir/OPENSSL-LICENSE.txt"
        libraries=("$install_dir"/lib/libtdjson*.dylib)
        if [[ ${#libraries[@]} -eq 0 ]]; then
            printf 'TDLib install contains no macOS shared library in %s\n' "$install_dir/lib" >&2
            exit 2
        fi
        cp -a "${libraries[@]}" "$package_dir/lib/"
        library=$(find "$package_dir/lib" -type f -name 'libtdjson*.dylib' -print -quit)
        otool -L "$library" > "$package_dir/DYNAMIC_DEPENDENCIES.txt"
        sha256_command=(shasum -a 256)
        ;;
    Linux)
        libraries=("$install_dir"/lib/libtdjson.so*)
        if [[ ${#libraries[@]} -eq 0 ]]; then
            printf 'TDLib install contains no Linux shared library in %s\n' "$install_dir/lib" >&2
            exit 2
        fi
        cp -a "${libraries[@]}" "$package_dir/lib/"
        library=$(find "$package_dir/lib" -type f -name 'libtdjson.so.*' -print -quit)
        ldd "$library" > "$package_dir/DYNAMIC_DEPENDENCIES.txt"
        sha256_command=(sha256sum)
        ;;
    *)
        printf 'unsupported packaging host: %s\n' "$(uname -s)" >&2
        exit 2
        ;;
esac

tar -C "$dist_dir" -cf - "$package_name" | zstd --threads=0 -19 -o "$archive"

sha256=$("${sha256_command[@]}" "$archive" | awk '{print $1}')

jq \
    --arg file "$(basename "$archive")" \
    --arg sha256 "$sha256" \
    '{file: $file} + del(.format_version) + {sha256: $sha256}' \
    <<< "$build_metadata" \
    > "$metadata"

printf '%s\n' "$archive"

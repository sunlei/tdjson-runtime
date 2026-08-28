#!/usr/bin/env bash

set -euo pipefail

if [[ $# -ne 6 ]]; then
    printf 'usage: %s <archive> <expected-version> <expected-commit> <expected-target> <new-extract-dir> <host-os>\n' "$0" >&2
    exit 2
fi

archive=$1
expected_version=$2
expected_commit=$3
expected_target=$4
extract_dir=$5
host_os=$6
short_commit=${expected_commit:0:12}
package_name="tdjson-v${expected_version}-g${short_commit}-${expected_target}"
package_dir="$extract_dir/$package_name"

if [[ "$(basename "$archive")" != "$package_name.tar.zst" ]]; then
    printf 'unexpected archive name: %s\n' "$archive" >&2
    exit 2
fi
if [[ -e "$extract_dir" ]]; then
    printf 'artifact extraction directory already exists: %s\n' "$extract_dir" >&2
    exit 2
fi

mkdir -p "$extract_dir"
zstd -d --stdout "$archive" | tar -C "$extract_dir" -xf -

jq -e \
    --arg version "$expected_version" \
    --arg commit "$expected_commit" \
    --arg target "$expected_target" \
    '.format_version == 1 and .tdlib_version == $version and .tdlib_commit == $commit and .target == $target' \
    "$package_dir/BUILD-METADATA.json" \
    > /dev/null

case "$host_os" in
    Linux)
        library=$(find "$package_dir/lib" -type f -name 'libtdjson.so.*' -print -quit)
        dependencies=$(ldd "$library")
        if [[ "$dependencies" == *'not found'* ]]; then
            printf 'artifact has unresolved dynamic dependencies:\n%s\n' "$dependencies" >&2
            exit 2
        fi
        ;;
    Darwin)
        library=$(find "$package_dir/lib" -type f -name 'libtdjson*.dylib' -print -quit)
        dependencies=$(otool -L "$library")
        dependencies=${dependencies#*$'\n'}
        is_library_id=true
        while read -r dependency _; do
            if "$is_library_id"; then
                if [[ "$dependency" != *libtdjson*.dylib ]]; then
                    printf 'macOS artifact has an unexpected library ID: %s\n' "$dependency" >&2
                    exit 2
                fi
                is_library_id=false
                continue
            fi
            case "$dependency" in
                /usr/lib/* | /System/Library/*)
                    ;;
                *)
                    printf 'macOS artifact has a non-system install name: %s\n' "$dependency" >&2
                    exit 2
                    ;;
            esac
        done <<< "$dependencies"
        if "$is_library_id"; then
            printf 'macOS artifact has no library ID\n' >&2
            exit 2
        fi
        ;;
    *)
        printf 'unsupported verification host: %s\n' "$host_os" >&2
        exit 2
        ;;
esac

printf '%s\n' "$library"

#!/usr/bin/env bash

set -euo pipefail

if [[ $# -lt 4 ]]; then
    printf 'usage: %s <upstream-commit> <expected-version> <new-source-dir> <new-install-dir> [cmake-option ...]\n' "$0" >&2
    exit 2
fi

upstream_commit=$1
expected_version=$2
source_dir=$3
install_dir=$4
shift 4

if [[ -e "$source_dir" ]]; then
    printf 'TDLib source directory already exists: %s\n' "$source_dir" >&2
    exit 2
fi
if [[ -e "$install_dir" ]]; then
    printf 'TDLib install directory already exists: %s\n' "$install_dir" >&2
    exit 2
fi

mkdir -p "$(dirname "$source_dir")" "$(dirname "$install_dir")"
git init "$source_dir"
git -C "$source_dir" remote add origin https://github.com/tdlib/td.git
git -C "$source_dir" fetch --depth 1 origin "$upstream_commit"
git -C "$source_dir" checkout --detach FETCH_HEAD

actual_commit=$(git -C "$source_dir" rev-parse HEAD)
actual_version=$(sed -nE 's/^project\(TDLib VERSION ([0-9]+\.[0-9]+\.[0-9]+) LANGUAGES CXX C\)$/\1/p' "$source_dir/CMakeLists.txt")
if [[ "$actual_commit" != "$upstream_commit" ]]; then
    printf 'TDLib commit mismatch: expected %s, got %s\n' "$upstream_commit" "$actual_commit" >&2
    exit 2
fi
if [[ "$actual_version" != "$expected_version" ]]; then
    printf 'TDLib version mismatch: expected %s, got %s\n' "$expected_version" "$actual_version" >&2
    exit 2
fi

case $(uname -s) in
    Darwin)
        parallelism=$(sysctl -n hw.logicalcpu)
        ;;
    Linux)
        parallelism=$(getconf _NPROCESSORS_ONLN)
        ;;
    *)
        printf 'unsupported build host: %s\n' "$(uname -s)" >&2
        exit 2
        ;;
esac

export CC=${CC:-clang}
export CXX=${CXX:-clang++}
export CXXFLAGS="${CXXFLAGS:+${CXXFLAGS} }-stdlib=libc++"

cmake \
    -S "$source_dir" \
    -B "$source_dir/build" \
    -G "Unix Makefiles" \
    -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$install_dir" \
    -DTD_INSTALL_STATIC_LIBRARIES=OFF \
    -DTD_INSTALL_SHARED_LIBRARIES=ON \
    -DBUILD_TESTING=OFF \
    "$@"
cmake --build "$source_dir/build" --target tdjson --parallel "$parallelism"
cmake --install "$source_dir/build"

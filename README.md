# tdjson-runtime

`tdjson-runtime` provides two closely related deliverables for the
[TDLib JSON interface](https://github.com/tdlib/td#using-from-other-programming-languages):

- immutable, prebuilt `libtdjson` release artifacts;
- a small Rust crate that loads the five current TDLib JSON C ABI functions at runtime.

The crate does not link TDLib during `cargo build`, download native libraries, generate the full
TDLib API, or choose an artifact for the caller. Applications must provide an explicit absolute
path to a trusted release artifact and decide which TDLib versions they support.

## Rust API

```rust,no_run
use std::path::Path;

use tdjson_runtime::NativeTdJson;

fn main() -> Result<(), tdjson_runtime::TdJsonError> {
    let path = Path::new("/opt/tdjson/lib/libtdjson.so.1.8.67");

    // SAFETY: the path names a trusted TDLib artifact whose ABI matches this crate.
    let mut tdjson = unsafe { NativeTdJson::load(path) }?;
    let identity = tdjson.build_identity()?;

    println!("TDLib {} ({})", identity.version(), identity.commit_hash());
    Ok(())
}
```

The first successful `NativeTdJson::load` permanently binds the process to that exact library path;
the library remains loaded until process exit. At most one thread-bound owner can exist at a time.
Its mutable methods serialize the process-global JSON interface, `receive` accepts a
`std::time::Duration`, and response bytes are copied before another native call can invalidate
them. Dropping the owner releases access but does not close TDLib clients.

The optional native log callback is removed when its owner is dropped, but removal does not wait
for an invocation already in progress. The callback and everything it accesses must therefore
remain valid until process exit. TDLib aborts after a verbosity-level-0 callback returns because
that level reports a fatal error.

## Release builds

The manually triggered `Release TDLib artifacts` workflow resolves `tdlib/td` `master` once and
builds that exact commit on all targets:

| Target | Build environment | Compiler | Generator |
|---|---|---|---|
| Linux x86_64 | Debian 13 container on a native x86_64 runner | Clang + libc++ | Unix Makefiles |
| Linux aarch64 | Debian 13 container on a native ARM64 runner | Clang + libc++ | Unix Makefiles |
| macOS arm64 | Apple Silicon runner | Apple Clang + libc++ | Unix Makefiles |

Only the shared `tdjson` target and its transitive dependencies are built. Archive names contain
the TDLib version and short commit. Releases contain the three `.tar.zst` archives, an aggregate
`manifest.json`, and `SHA256SUMS`. Every archive includes build metadata, the installed shared
library, TDLib's license, and its dynamic dependency report. The macOS archive also includes the
license for its statically linked OpenSSL.

Linux artifacts are validated in a fresh Debian 13 slim consumer with the runtime packages
`libc++1`, `libc++abi1`, `libunwind-19`, `libssl3t64`, and `zlib1g` installed. Compatibility with
other distributions is not implied; use the archive's dependency report to evaluate another
consumer environment. macOS artifacts statically link Homebrew OpenSSL and are rejected unless all
remaining install names refer to the artifact itself or Apple system libraries.

Both consumer checks unpack the final archive and run the native identity, callback, and client
lifecycle test against the unpacked library. The release job also requires all three targets and
reconciles each archive hash with its metadata before publishing.

Before the first workflow run, enable release immutability in the repository settings. GitHub then
locks the tag and assets when the draft is published. The workflow does not mark any release as
`latest`; consumers select and verify a specific release identity.

## Development

```shell
cargo fmt --check
cargo clippy --release --all-targets -- -D warnings
cargo nextest run --release
```

Native tests are ignored by default. Run them only with a library built from the expected source:

```shell
TDJSON_LIBRARY=/absolute/path/to/libtdjson \
TDJSON_EXPECTED_VERSION=1.8.67 \
TDJSON_EXPECTED_COMMIT=<full-commit> \
cargo nextest run --release --locked --run-ignored ignored-only --test native
```

## License

This repository does not currently declare a license. Packaged TDLib files retain TDLib's upstream
license, which is included in every native artifact.

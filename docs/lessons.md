# Engineering lessons

- A runtime-loaded native library with process-global state or background threads cannot safely
  tie `dlclose` to a Rust access owner's `Drop`. Bind the process once, retain the library for the
  process lifetime, and model owner lifetime only as exclusive access.
- Validate release archives after extraction in the declared consumer environment. Checks against
  a build install tree cannot detect missing files, unresolved runtime dependencies, or embedded
  loader paths in the published archive.
- Keep raw protocol passthrough APIs raw, and interpret structured error responses in typed helpers
  so diagnostics are preserved without changing the low-level response contract.

use std::path::Path;

use tdjson_runtime::{NativeTdJson, TdJsonError};

#[test]
fn rejects_relative_library_paths_before_loading() {
    // SAFETY: the relative path is rejected before any native library is loaded.
    let result = unsafe { NativeTdJson::load(Path::new("libtdjson.so")) };

    assert!(matches!(
        result,
        Err(TdJsonError::RelativeLibraryPath { .. })
    ));
}

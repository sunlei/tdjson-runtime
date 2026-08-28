use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use tdjson_runtime::{NativeTdJson, TdJsonError};

const LOG_MARKER: &[u8] = b"tdjson-runtime-native-callback";
static CALLBACK_COUNT: AtomicUsize = AtomicUsize::new(0);

#[test]
#[ignore = "requires explicit TDJSON_LIBRARY, TDJSON_EXPECTED_VERSION, and TDJSON_EXPECTED_COMMIT"]
fn loads_identity_callback_and_client_lifecycle() {
    let path = std::env::var_os("TDJSON_LIBRARY").expect("TDJSON_LIBRARY must be set");
    let expected_version =
        std::env::var("TDJSON_EXPECTED_VERSION").expect("TDJSON_EXPECTED_VERSION must be set");
    let expected_commit =
        std::env::var("TDJSON_EXPECTED_COMMIT").expect("TDJSON_EXPECTED_COMMIT must be set");
    CALLBACK_COUNT.store(0, Ordering::Relaxed);

    // SAFETY: the workflow provides the artifact built earlier in the same job from the expected
    // source commit, and this test process has no other TDLib caller.
    let mut tdjson = unsafe { NativeTdJson::load(Path::new(&path)) }
        .expect("TDLib shared library must load with all five JSON ABI symbols");
    let identity = tdjson
        .build_identity()
        .expect("loaded TDLib must report its build identity");

    assert_eq!(identity.version(), expected_version);
    assert_eq!(identity.commit_hash(), expected_commit);

    // SAFETY: this attempts to load the same trusted artifact while the first owner is active.
    let second_owner = unsafe { NativeTdJson::load(Path::new(&path)) };

    assert!(matches!(second_owner, Err(TdJsonError::OwnerAlreadyActive)));

    // SAFETY: the callback and its static state remain valid until process exit. It does not panic,
    // retain the message pointer, or invoke TDLib.
    unsafe {
        tdjson
            .install_log_message_callback(1, record_log_message)
            .expect("the process-global callback must install once");
    }
    // SAFETY: this is the same process-lifetime callback; the duplicate is rejected before TDLib is
    // called.
    let duplicate_callback = unsafe { tdjson.install_log_message_callback(1, record_log_message) };

    assert!(matches!(
        duplicate_callback,
        Err(TdJsonError::LogCallbackAlreadyInstalled)
    ));

    let response = tdjson
        .execute(
            br#"{"@type":"addLogMessage","verbosity_level":1,"text":"tdjson-runtime-native-callback"}"#,
        )
        .expect("addLogMessage must execute synchronously");
    let response: Value = serde_json::from_slice(&response).expect("response must be valid JSON");

    assert_eq!(response["@type"], "ok");
    assert_eq!(CALLBACK_COUNT.load(Ordering::Relaxed), 1);

    let client_id = tdjson.create_client_id();
    tdjson
        .send(client_id, br#"{"@type":"close"}"#)
        .expect("close request must be accepted");
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut closed = false;
    while Instant::now() < deadline {
        let Some(frame) = tdjson.receive(Duration::from_millis(250)) else {
            continue;
        };
        let frame: Value = serde_json::from_slice(&frame).expect("TDLib frame must be valid JSON");
        if frame["@client_id"] == client_id
            && frame["@type"] == "updateAuthorizationState"
            && frame["authorization_state"]["@type"] == "authorizationStateClosed"
        {
            closed = true;
            break;
        }
    }

    assert!(closed, "client must reach authorizationStateClosed");
    drop(tdjson);

    let different_path = Path::new(&path).with_file_name("different-libtdjson");
    // SAFETY: the process is already bound, so the different path is rejected before loading.
    let different_library = unsafe { NativeTdJson::load(&different_path) };

    assert!(matches!(
        different_library,
        Err(TdJsonError::LibraryPathMismatch { .. })
    ));

    // SAFETY: the same trusted, permanently resident artifact is reacquired after the first owner
    // removed its callback and released process-global access.
    let mut second = unsafe { NativeTdJson::load(Path::new(&path)) }
        .expect("the resident library must be available after the first owner is dropped");
    // SAFETY: the callback and its static state remain valid until process exit. The first owner
    // removed the callback before releasing process-global access.
    unsafe {
        second
            .install_log_message_callback(1, record_log_message)
            .expect("callback ownership must be released on Drop");
    }
    let response = second
        .execute(
            br#"{"@type":"addLogMessage","verbosity_level":1,"text":"tdjson-runtime-native-callback"}"#,
        )
        .expect("addLogMessage must execute after callback ownership is reacquired");
    let response: Value = serde_json::from_slice(&response).expect("response must be valid JSON");

    assert_eq!(response["@type"], "ok");
    assert_eq!(CALLBACK_COUNT.load(Ordering::Relaxed), 2);
}

unsafe extern "C" fn record_log_message(_verbosity_level: c_int, message: *const c_char) {
    if message.is_null() {
        return;
    }
    // SAFETY: TDLib supplies a null-terminated callback string that is valid for this invocation.
    let message = unsafe { CStr::from_ptr(message) }.to_bytes();
    if message
        .windows(LOG_MARKER.len())
        .any(|window| window == LOG_MARKER)
    {
        CALLBACK_COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

//! Runtime loading and ownership boundary for the TDLib JSON C interface.
#![doc = include_str!("../README.md")]

use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::os::raw::{c_char, c_double, c_int};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str::Utf8Error;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use libloading::Library;
use serde_json::Value;
use thiserror::Error;

type TdCreateClientId = unsafe extern "C" fn() -> c_int;
type TdSend = unsafe extern "C" fn(c_int, *const c_char);
type TdReceive = unsafe extern "C" fn(c_double) -> *const c_char;
type TdExecute = unsafe extern "C" fn(*const c_char) -> *const c_char;
type TdSetLogMessageCallback = unsafe extern "C" fn(c_int, Option<TdLogMessageCallback>);

/// Callback type accepted by `td_set_log_message_callback`.
pub type TdLogMessageCallback = unsafe extern "C" fn(c_int, *const c_char);

static OWNER_ACTIVE: AtomicBool = AtomicBool::new(false);
static TDJSON_BINDING: Mutex<Option<TdJsonBinding>> = Mutex::new(None);

/// Source identity reported by the loaded TDLib library.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TdLibBuildIdentity {
    version: String,
    commit_hash: String,
}

impl TdLibBuildIdentity {
    fn new(version: String, commit_hash: String) -> Self {
        Self {
            version,
            commit_hash,
        }
    }

    /// Returns the TDLib semantic version.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns the full upstream Git commit reported by TDLib.
    pub fn commit_hash(&self) -> &str {
        &self.commit_hash
    }
}

/// Errors raised at the dynamic-library and TDLib JSON trust boundaries.
#[derive(Debug, Error)]
pub enum TdJsonError {
    /// The caller supplied a relative library path.
    #[error("TDLib shared library path must be absolute: {path}")]
    RelativeLibraryPath { path: PathBuf },
    /// The operating-system loader rejected the shared library.
    #[error("failed to load TDLib shared library at {path}")]
    LoadLibrary {
        path: PathBuf,
        #[source]
        source: libloading::Error,
    },
    /// Another `NativeTdJson` currently owns the process-global JSON interface.
    #[error("TDLib's process-global JSON interface already has an active owner")]
    OwnerAlreadyActive,
    /// The process was already permanently bound to a different TDLib library path.
    #[error(
        "TDLib is already bound to {loaded_path}; cannot bind the same process to {requested_path}"
    )]
    LibraryPathMismatch {
        loaded_path: PathBuf,
        requested_path: PathBuf,
    },
    /// The library does not expose one of the required JSON ABI functions.
    #[error("TDLib shared library is missing symbol {symbol}")]
    MissingSymbol {
        symbol: &'static str,
        #[source]
        source: libloading::Error,
    },
    /// A JSON request contains an interior NUL byte and cannot be passed to C.
    #[error("TDLib request contains an interior NUL byte")]
    RequestContainsNul(#[source] std::ffi::NulError),
    /// This owner has already installed TDLib's process-global log callback.
    #[error("TDLib native log callback is already installed")]
    LogCallbackAlreadyInstalled,
    /// `td_execute` unexpectedly returned a null pointer.
    #[error("td_execute returned a null pointer")]
    NullExecuteResponse,
    /// A synchronous TDLib response was not UTF-8.
    #[error("td_execute returned non-UTF-8 bytes")]
    InvalidUtf8(#[source] Utf8Error),
    /// A synchronous TDLib response was not valid JSON.
    #[error("td_execute returned invalid JSON")]
    InvalidJson(#[source] serde_json::Error),
    /// `getOption` did not return the documented string variant.
    #[error("getOption({option}) did not return optionValueString with a string value")]
    InvalidOptionResponse { option: &'static str },
}

/// Single-threaded owner of one dynamically loaded TDLib JSON library.
///
/// Every client created through this interface must reach `authorizationStateClosed` before
/// process termination. Dropping the owner releases access but does not close clients or unload the
/// permanently resident library.
pub struct NativeTdJson {
    symbols: TdJsonSymbols,
    log_callback_installed: bool,
    _thread_bound: PhantomData<Rc<()>>,
}

impl NativeTdJson {
    /// Loads a caller-selected TDLib shared library from an absolute path.
    ///
    /// # Safety
    ///
    /// `path` must identify a trusted library whose exported functions match TDLib's current JSON
    /// C ABI. Its initialization code must be safe to execute in this process. The first successful
    /// call permanently binds the process to this exact path and keeps the library loaded until
    /// process exit. No code may bypass this owner and call the library's JSON C ABI directly.
    pub unsafe fn load(path: impl AsRef<Path>) -> Result<Self, TdJsonError> {
        let path = path.as_ref();
        if !path.is_absolute() {
            return Err(TdJsonError::RelativeLibraryPath {
                path: path.to_path_buf(),
            });
        }

        let mut binding = TDJSON_BINDING.lock().unwrap();
        if OWNER_ACTIVE.load(Ordering::Acquire) {
            return Err(TdJsonError::OwnerAlreadyActive);
        }

        let symbols = if let Some(binding) = binding.as_ref() {
            if binding.path != path {
                return Err(TdJsonError::LibraryPathMismatch {
                    loaded_path: binding.path.clone(),
                    requested_path: path.to_path_buf(),
                });
            }
            binding.symbols
        } else {
            // SAFETY: the caller accepts the native initialization contract documented above.
            let library =
                unsafe { Library::new(path) }.map_err(|source| TdJsonError::LoadLibrary {
                    path: path.to_path_buf(),
                    source,
                })?;
            let symbols = load_symbols(&library)?;
            *binding = Some(TdJsonBinding {
                _library: library,
                path: path.to_path_buf(),
                symbols,
            });
            symbols
        };

        let owner_was_active = OWNER_ACTIVE.swap(true, Ordering::AcqRel);
        assert!(
            !owner_was_active,
            "TDLib owner state changed while load was locked"
        );
        Ok(Self {
            symbols,
            log_callback_installed: false,
            _thread_bound: PhantomData,
        })
    }

    /// Creates a client identifier owned by TDLib's global JSON interface.
    pub fn create_client_id(&mut self) -> c_int {
        // SAFETY: symbol loading established the function signature; mutable access serializes the
        // native boundary owned by this value.
        unsafe { (self.symbols.create_client_id)() }
    }

    /// Sends an already serialized JSON request to a TDLib client.
    pub fn send(&mut self, client_id: c_int, request: &[u8]) -> Result<(), TdJsonError> {
        let request = CString::new(request).map_err(TdJsonError::RequestContainsNul)?;
        // SAFETY: the symbol signature is fixed and the CString lives through the call.
        unsafe { (self.symbols.send)(client_id, request.as_ptr()) };
        Ok(())
    }

    /// Receives one frame and copies TDLib-owned bytes before a later native call invalidates them.
    pub fn receive(&mut self, timeout: Duration) -> Option<Vec<u8>> {
        // SAFETY: mutable access prevents concurrent receive/execute calls through this owner.
        let response = unsafe { (self.symbols.receive)(timeout.as_secs_f64()) };
        copy_native_response(response)
    }

    /// Executes an already serialized synchronous JSON request.
    pub fn execute(&mut self, request: &[u8]) -> Result<Vec<u8>, TdJsonError> {
        let request = CString::new(request).map_err(TdJsonError::RequestContainsNul)?;
        // SAFETY: mutable access prevents concurrent receive/execute calls through this owner. The
        // returned bytes are copied before this method permits another native call.
        let response = unsafe { (self.symbols.execute)(request.as_ptr()) };
        copy_native_response(response).ok_or(TdJsonError::NullExecuteResponse)
    }

    /// Queries the version and full upstream commit reported by the loaded library.
    pub fn build_identity(&mut self) -> Result<TdLibBuildIdentity, TdJsonError> {
        Ok(TdLibBuildIdentity::new(
            self.get_string_option("version", br#"{"@type":"getOption","name":"version"}"#)?,
            self.get_string_option(
                "commit_hash",
                br#"{"@type":"getOption","name":"commit_hash"}"#,
            )?,
        ))
    }

    /// Installs TDLib's process-global native log callback.
    ///
    /// # Safety
    ///
    /// `callback` and everything it accesses must remain valid until process exit because removing
    /// the callback is not a synchronization barrier for invocations already in progress. It must
    /// not panic, retain the message pointer, or call any TDLib method. TDLib can invoke it from
    /// native threads. A message with verbosity level 0 is TDLib's fatal path; TDLib aborts after
    /// the callback returns.
    pub unsafe fn install_log_message_callback(
        &mut self,
        max_verbosity_level: c_int,
        callback: TdLogMessageCallback,
    ) -> Result<(), TdJsonError> {
        if self.log_callback_installed {
            return Err(TdJsonError::LogCallbackAlreadyInstalled);
        }

        // SAFETY: the caller accepts the callback contract documented above.
        unsafe {
            (self.symbols.set_log_message_callback)(max_verbosity_level, Some(callback));
        }
        self.log_callback_installed = true;
        Ok(())
    }

    fn get_string_option(
        &mut self,
        option: &'static str,
        request: &'static [u8],
    ) -> Result<String, TdJsonError> {
        let response = self.execute(request)?;
        std::str::from_utf8(&response).map_err(TdJsonError::InvalidUtf8)?;
        let response: Value =
            serde_json::from_slice(&response).map_err(TdJsonError::InvalidJson)?;
        if response.get("@type").and_then(Value::as_str) != Some("optionValueString") {
            return Err(TdJsonError::InvalidOptionResponse { option });
        }
        response
            .get("value")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or(TdJsonError::InvalidOptionResponse { option })
    }
}

impl Drop for NativeTdJson {
    fn drop(&mut self) {
        if self.log_callback_installed {
            // SAFETY: the permanently resident binding keeps the function pointer valid. This
            // prevents new callback invocations but does not wait for an invocation in progress.
            unsafe { (self.symbols.set_log_message_callback)(0, None) };
            self.log_callback_installed = false;
        }

        OWNER_ACTIVE.store(false, Ordering::Release);
    }
}

struct TdJsonBinding {
    // Static values are never dropped, so retaining the binding prevents dlclose while TDLib native
    // threads or process-global state can still refer to code and data from this library.
    _library: Library,
    path: PathBuf,
    symbols: TdJsonSymbols,
}

#[derive(Clone, Copy)]
struct TdJsonSymbols {
    create_client_id: TdCreateClientId,
    send: TdSend,
    receive: TdReceive,
    execute: TdExecute,
    set_log_message_callback: TdSetLogMessageCallback,
}

fn load_symbols(library: &Library) -> Result<TdJsonSymbols, TdJsonError> {
    // SAFETY: every type below mirrors the corresponding declaration in td_json_client.h. The
    // process-global binding retains Library for longer than the copied function pointers.
    unsafe {
        Ok(TdJsonSymbols {
            create_client_id: *library.get(b"td_create_client_id\0").map_err(|source| {
                TdJsonError::MissingSymbol {
                    symbol: "td_create_client_id",
                    source,
                }
            })?,
            send: *library
                .get(b"td_send\0")
                .map_err(|source| TdJsonError::MissingSymbol {
                    symbol: "td_send",
                    source,
                })?,
            receive: *library.get(b"td_receive\0").map_err(|source| {
                TdJsonError::MissingSymbol {
                    symbol: "td_receive",
                    source,
                }
            })?,
            execute: *library.get(b"td_execute\0").map_err(|source| {
                TdJsonError::MissingSymbol {
                    symbol: "td_execute",
                    source,
                }
            })?,
            set_log_message_callback: *library.get(b"td_set_log_message_callback\0").map_err(
                |source| TdJsonError::MissingSymbol {
                    symbol: "td_set_log_message_callback",
                    source,
                },
            )?,
        })
    }
}

fn copy_native_response(response: *const c_char) -> Option<Vec<u8>> {
    if response.is_null() {
        return None;
    }

    // SAFETY: TDLib promises a non-null response is a null-terminated string valid until the next
    // receive or execute call. The owned copy is completed before returning to the caller.
    Some(unsafe { CStr::from_ptr(response) }.to_bytes().to_owned())
}

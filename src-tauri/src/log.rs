//! Problems the app recovers from. They go to stderr and, once the app has called `to_file`, to
//! `omega.log` in the office folder, since a Windows release build has no console.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Past this size the log starts over when the app opens.
const MAX_BYTES: u64 = 1024 * 1024;

static FILE: OnceLock<Mutex<File>> = OnceLock::new();

pub fn to_file(path: &Path) -> std::io::Result<()> {
    let big = std::fs::metadata(path).is_ok_and(|m| m.len() > MAX_BYTES);
    let file = OpenOptions::new().create(true).append(!big).write(true).truncate(big).open(path)?;
    let _ = FILE.set(Mutex::new(file));
    Ok(())
}

fn stamped(message: std::fmt::Arguments) -> String {
    format!("{} {message}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))
}

pub fn write(message: std::fmt::Arguments) {
    let line = stamped(message);
    eprintln!("{line}");
    if let Some(f) = FILE.get() {
        let mut f = f.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(f, "{line}");
    }
}

/// Writes the cause of a panic to the log before the process goes (release builds abort on panic).
/// On macOS it also records the last Objective-C exception thrown, since AppKit can panic after one.
pub fn catch_crashes() {
    #[cfg(target_os = "macos")]
    objc::record_exceptions();
    std::panic::set_hook(Box::new(|info| {
        #[cfg(target_os = "macos")]
        let exception = objc::last_exception();
        #[cfg(not(target_os = "macos"))]
        let exception: Option<String> = None;
        let line = match exception {
            Some(e) => stamped(format_args!("crashed: {info} (the last Objective-C exception before it was {e})")),
            None => stamped(format_args!("crashed: {info}")),
        };
        eprintln!("{line}");
        // A panic while the log is locked must not wait for it.
        if let Some(mut f) = FILE.get().and_then(|f| f.try_lock().ok()) {
            let _ = writeln!(f, "{line}");
        }
    }));
}

#[cfg(target_os = "macos")]
mod objc {
    use objc2::runtime::AnyObject;
    use objc2_foundation::NSException;
    use std::sync::{Mutex, OnceLock};

    type Preprocessor = unsafe extern "C" fn(*mut AnyObject) -> *mut AnyObject;

    /// `name: reason` of the last exception thrown. AppKit throws and catches exceptions in normal
    /// use, so this is only a clue, read when the app panics.
    static LAST: Mutex<Option<String>> = Mutex::new(None);
    static PREVIOUS: OnceLock<Preprocessor> = OnceLock::new();

    pub fn record_exceptions() {
        // SAFETY: `record` keeps the runtime's contract: it hands back the exception it was given.
        let previous = unsafe { objc2::ffi::objc_setExceptionPreprocessor(record) };
        let _ = PREVIOUS.set(previous);
    }

    pub fn last_exception() -> Option<String> {
        LAST.try_lock().ok().and_then(|l| l.clone())
    }

    unsafe extern "C" fn record(exception: *mut AnyObject) -> *mut AnyObject {
        // SAFETY: the runtime passes the object being thrown, alive for the whole call.
        if let Some(e) = unsafe { exception.as_ref() }.and_then(|o| o.downcast_ref::<NSException>()) {
            let reason = e.reason().map(|r| r.to_string()).unwrap_or_default();
            if let Ok(mut last) = LAST.try_lock() {
                *last = Some(format!("{}: {reason}", e.name()));
            }
        }
        match PREVIOUS.get() {
            // SAFETY: the preprocessor installed before ours, called as the runtime would.
            Some(previous) => unsafe { previous(exception) },
            None => exception,
        }
    }
}

/// `log!("...", args)`: like `eprintln!`, and also into `omega.log`.
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(format_args!($($t)*)) };
}
pub(crate) use log;

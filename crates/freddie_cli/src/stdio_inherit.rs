//! Keep the client's stdio out of a spawned daemon.

use std::io;

/// Restores this process's stdin/stdout/stderr inherit flags when dropped.
///
/// `Command::spawn` on Windows inherits every inheritable handle, not only the three
/// `Stdio` values. A client's `output()` pipe is inheritable, so the daemon would keep
/// it open and `output()` would never see EOF. Clearing `HANDLE_FLAG_INHERIT` on the
/// parent's standard handles for the spawn leaves the child's `Stdio::null()` handles
/// inheritable (they are newly opened) and the client's pipe not.
#[cfg(windows)]
pub(crate) struct IsolateParentStdio {
    handles: [Option<RestoredHandle>; 3],
}

#[cfg(not(windows))]
pub(crate) struct IsolateParentStdio;

#[cfg(windows)]
struct RestoredHandle {
    handle: windows_sys::Win32::Foundation::HANDLE,
    inherit: HandleInherit,
}

#[cfg(windows)]
enum HandleInherit {
    Inheritable,
    Isolated,
}

impl IsolateParentStdio {
    #[cfg_attr(not(windows), expect(clippy::unnecessary_wraps))]
    pub(crate) fn enter() -> io::Result<Self> {
        #[cfg(not(windows))]
        {
            Ok(Self)
        }
        #[cfg(windows)]
        {
            Self::enter_windows()
        }
    }

    #[cfg(windows)]
    fn enter_windows() -> io::Result<Self> {
        use windows_sys::Win32::Foundation::{
            GetHandleInformation, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, SetHandleInformation,
        };
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
        };

        let ids = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE];
        let mut guard = Self {
            handles: [None, None, None],
        };
        for (i, id) in ids.into_iter().enumerate() {
            #[expect(unsafe_code)]
            // SAFETY: GetStdHandle reads this process's standard handle table and does not
            // dereference the handle.
            let handle = unsafe { GetStdHandle(id) };
            if handle.is_null() || handle == INVALID_HANDLE_VALUE {
                continue;
            }
            let mut flags = 0u32;
            #[expect(unsafe_code)]
            // SAFETY: `handle` came from GetStdHandle and is neither null nor invalid.
            let ok = unsafe { GetHandleInformation(handle, &raw mut flags) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            let inherit = if flags & HANDLE_FLAG_INHERIT == 0 {
                HandleInherit::Isolated
            } else {
                HandleInherit::Inheritable
            };
            #[expect(unsafe_code)]
            // SAFETY: same handle. Clearing HANDLE_FLAG_INHERIT does not close it.
            let ok = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) };
            if ok == 0 {
                return Err(io::Error::last_os_error());
            }
            guard.handles[i] = Some(RestoredHandle { handle, inherit });
        }
        Ok(guard)
    }
}

#[cfg(windows)]
impl Drop for IsolateParentStdio {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::{HANDLE_FLAG_INHERIT, SetHandleInformation};

        for restored in &self.handles {
            let Some(restored) = restored else {
                continue;
            };
            let flags = match restored.inherit {
                HandleInherit::Inheritable => HANDLE_FLAG_INHERIT,
                HandleInherit::Isolated => 0,
            };
            #[expect(unsafe_code)]
            // SAFETY: `handle` was taken from GetStdHandle at enter and is still this
            // process's standard handle. Restore is best-effort; the spawn has finished.
            let _ = unsafe { SetHandleInformation(restored.handle, HANDLE_FLAG_INHERIT, flags) };
        }
    }
}

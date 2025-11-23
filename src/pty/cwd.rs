// packages/kodegen-tools-terminal/src/pty/cwd.rs
//! Cross-platform current working directory lookup for PTY processes.
//!
//! This module provides OS-specific implementations to query the current working
//! directory of a PTY child process using its PID. The implementation is based on
//! alacritty's approach for daemon process CWD tracking.

use std::io;
use std::path::PathBuf;

/// Get current working directory of a process by PID.
///
/// This queries the operating system for the process's CWD using platform-specific APIs:
/// - **Linux**: Reads `/proc/{pid}/cwd` symlink
/// - **macOS**: Uses `proc_pidinfo` syscall with `PROC_PIDVNODEPATHINFO`
///
/// # Arguments
/// * `pid` - Process ID of the PTY child process
///
/// # Returns
/// * `Ok(PathBuf)` - Current working directory path
/// * `Err(io::Error)` - Failed to query CWD (process died, permission denied, etc.)
///
/// # References
/// Based on alacritty's implementation:
/// - Linux: `alacritty/src/daemon.rs:94-117`
/// - macOS: `alacritty/src/macos/proc.rs:54-69`
pub fn get_cwd(pid: u32) -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{}/cwd", pid))
    }

    #[cfg(target_os = "macos")]
    {
        macos::get_cwd_macos(pid)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "CWD tracking not supported on this platform",
        ))
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use std::ffi::CStr;
    use std::io;
    use std::mem::{self, MaybeUninit};
    use std::path::PathBuf;

    // macOS proc_info types and constants
    use libc::{c_int, c_void, proc_pidinfo, proc_vnodepathinfo};

    const PROC_PIDVNODEPATHINFO: c_int = 9;

    /// Get CWD on macOS using proc_pidinfo syscall
    pub fn get_cwd_macos(pid: u32) -> io::Result<PathBuf> {
        let mut info = MaybeUninit::<proc_vnodepathinfo>::uninit();
        let info_ptr = info.as_mut_ptr() as *mut c_void;
        let size = mem::size_of::<proc_vnodepathinfo>() as c_int;

        let c_str = unsafe {
            let pidinfo_size =
                proc_pidinfo(pid as c_int, PROC_PIDVNODEPATHINFO, 0, info_ptr, size);
            if pidinfo_size < 0 {
                return Err(io::Error::last_os_error());
            }
            if pidinfo_size != size {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Invalid proc_pidinfo return size",
                ));
            }
            CStr::from_ptr(info.assume_init().pvi_cdir.vip_path.as_ptr() as *const i8)
        };

        let path_str = c_str
            .to_str()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        Ok(PathBuf::from(path_str))
    }
}

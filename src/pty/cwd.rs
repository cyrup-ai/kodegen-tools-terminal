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
/// - **Windows**: Reads PEB via `NtQueryInformationProcess` + `ReadProcessMemory`
///
/// # Arguments
/// * `pid` - Process ID of the PTY child process
///
/// # Returns
/// * `Ok(PathBuf)` - Current working directory path
/// * `Err(io::Error)` - Failed to query CWD (process died, permission denied, etc.)
///
/// # References
/// Based on alacritty's implementation (Linux/macOS only - Windows is a kodegen extension):
/// - Linux: `alacritty/src/daemon.rs:94-117`
/// - macOS: `alacritty/src/macos/proc.rs:54-69`
/// - Windows: Uses undocumented NT APIs to read process PEB
pub fn get_cwd(pid: u32) -> io::Result<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_link(format!("/proc/{}/cwd", pid))
    }

    #[cfg(target_os = "macos")]
    {
        macos::get_cwd_macos(pid)
    }

    #[cfg(target_os = "windows")]
    {
        windows::get_cwd_windows(pid)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = pid; // Silence unused variable warning on unsupported platforms
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

#[cfg(target_os = "windows")]
mod windows {
    use std::io;
    use std::mem;
    use std::path::PathBuf;
    use std::ptr;

    use ntapi::ntpsapi::{NtQueryInformationProcess, ProcessBasicInformation, PROCESS_BASIC_INFORMATION};
    use ntapi::ntrtl::RTL_USER_PROCESS_PARAMETERS;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_VM_READ};
    use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;

    // ============================================================================
    // PEB OFFSET CONSTANTS (Architecture-Dependent)
    // ============================================================================
    
    /// Offset of ProcessParameters field in PEB structure (64-bit Windows)
    /// 
    /// **Reference**: 
    /// - Geoff Chappell: https://www.geoffchappell.com/studies/windows/km/ntoskrnl/inc/api/pebteb/peb/index.htm
    /// - Travis Mathison: https://www.travismathison.com/posts/PEB_TEB_TIB-Structure-Offsets/
    /// 
    /// **Stability**: This offset has been stable since Windows XP x64 through Windows 11.
    /// Microsoft rarely changes PEB layout, but this is undocumented and subject to change.
    #[cfg(target_pointer_width = "64")]
    const PEB_PROCESS_PARAMS_OFFSET: usize = 0x20;

    /// Offset of ProcessParameters field in PEB structure (32-bit Windows)
    /// 
    /// **Reference**: 
    /// - Geoff Chappell: https://www.geoffchappell.com/studies/windows/km/ntoskrnl/inc/api/pebteb/peb/index.htm
    /// - Travis Mathison: https://www.travismathison.com/posts/PEB_TEB_TIB-Structure-Offsets/
    /// 
    /// **Stability**: This offset has been stable since Windows XP through Windows 10 32-bit.
    #[cfg(target_pointer_width = "32")]
    const PEB_PROCESS_PARAMS_OFFSET: usize = 0x10;

    // Compile-time validation: only support known architectures
    #[cfg(not(any(target_pointer_width = "32", target_pointer_width = "64")))]
    compile_error!(
        "Windows CWD lookup only supports 32-bit and 64-bit platforms. \
         PEB structure offsets are architecture-dependent and unknown for this platform."
    );

    /// Get CWD on Windows by reading the PEB (Process Environment Block)
    pub fn get_cwd_windows(pid: u32) -> io::Result<PathBuf> {
        unsafe {
            // Open process with query and read permissions
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, 0, pid);
            if handle == ptr::null_mut() {
                return Err(io::Error::last_os_error());
            }

            // Ensure handle is closed on drop
            let _guard = HandleGuard(handle);

            // Query process basic information to get PEB address
            let mut pbi: PROCESS_BASIC_INFORMATION = mem::zeroed();
            let mut return_length = 0u32;
            let status = NtQueryInformationProcess(
                handle as *mut _,
                ProcessBasicInformation,
                &mut pbi as *mut _ as *mut _,
                mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                &mut return_length,
            );

            if status < 0 {
                return Err(io::Error::from_raw_os_error(status));
            }

            // Read ProcessParameters pointer from PEB using architecture-specific offset
            // - 32-bit Windows: PEB + 0x10
            // - 64-bit Windows: PEB + 0x20
            let mut process_params_ptr: *mut RTL_USER_PROCESS_PARAMETERS = ptr::null_mut();
            let peb_addr = (pbi.PebBaseAddress as usize + PEB_PROCESS_PARAMS_OFFSET) as *const _;

            if ReadProcessMemory(
                handle,
                peb_addr,
                &mut process_params_ptr as *mut _ as *mut _,
                mem::size_of::<*mut RTL_USER_PROCESS_PARAMETERS>(),
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }

            // Read ProcessParameters structure
            let mut process_params: RTL_USER_PROCESS_PARAMETERS = mem::zeroed();
            if ReadProcessMemory(
                handle,
                process_params_ptr as *const _,
                &mut process_params as *mut _ as *mut _,
                mem::size_of::<RTL_USER_PROCESS_PARAMETERS>(),
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }

            // Read CurrentDirectory.DosPath string
            let cwd_length = process_params.CurrentDirectory.DosPath.Length as usize;
            if cwd_length == 0 {
                return Err(io::Error::new(io::ErrorKind::NotFound, "No CWD found"));
            }

            let mut cwd_buffer: Vec<u16> = vec![0; cwd_length / 2 + 1];
            if ReadProcessMemory(
                handle,
                process_params.CurrentDirectory.DosPath.Buffer as *const _,
                cwd_buffer.as_mut_ptr() as *mut _,
                cwd_length,
                ptr::null_mut(),
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }

            // Convert wide string to PathBuf
            let cwd_str = String::from_utf16_lossy(&cwd_buffer[..cwd_length / 2]);
            Ok(PathBuf::from(cwd_str))
        }
    }

    struct HandleGuard(HANDLE);

    impl Drop for HandleGuard {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

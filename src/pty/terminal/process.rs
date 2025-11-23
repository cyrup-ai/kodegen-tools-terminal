use std::io;
use tokio::time::{Duration, timeout};

use super::types::Terminal;

impl Terminal {
    /// Close the terminal and kill the child process
    ///
    /// This method:
    /// - Signals the event loop to stop by dropping the sender
    /// - Waits for the event loop thread to complete
    ///
    /// For clean shutdown, call this method explicitly before dropping the Terminal.
    /// The Drop implementation provides best-effort cleanup but cannot await.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Event loop thread panicked during execution
    /// - Thread failed to join properly
    /// - Timeout (after 5 seconds)
    pub async fn close(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Drop sender to signal event loop shutdown
        // The event loop's channel receiver will return None, cleanly exiting the loop
        drop(self.sender.take());

        // Wait for event loop thread to finish (it owns the PTY and will clean it up)
        log::debug!("Waiting for event loop thread to finish");
        if let Some(handle) = self.event_loop_thread.take() {
            let timeout_duration = Duration::from_secs(5);
            match timeout(
                timeout_duration,
                tokio::task::spawn_blocking(move || handle.join())
            ).await {
                Ok(Ok(Ok(()))) => {
                    log::debug!("Event loop thread completed successfully");
                }
                Ok(Ok(Err(_))) => {
                    log::error!("Event loop thread panicked");
                    return Err("Event loop thread panicked".into());
                }
                Ok(Err(e)) => {
                    log::error!("Failed to join event loop thread: {}", e);
                    return Err(e.into());
                }
                Err(_) => {
                    log::error!("Event loop thread timeout after 5s - forcing drop");
                    return Err("Timeout waiting for event loop".into());
                }
            }
        }

        Ok(())
    }

    /// Wait for child process to exit and return exit status
    ///
    /// Note: Since the PTY is owned by the event loop thread, we poll the pty_closed flag.
    /// Exit code is not available - returns 0 when process exits.
    pub async fn wait(&mut self) -> io::Result<i32> {
        // Poll pty_closed flag until process exits
        while !self.is_pty_closed() {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        }
        Ok(0) // Exit code not available (PTY owned by event loop)
    }

    /// Try to get exit status without waiting (non-blocking)
    ///
    /// Note: Since the PTY is owned by the event loop thread, we check the pty_closed flag.
    /// Exit code is not available - returns Some(0) when process exits, None if still running.
    pub async fn try_wait(&mut self) -> io::Result<Option<i32>> {
        if self.is_pty_closed() {
            Ok(Some(0)) // Exit code not available (PTY owned by event loop)
        } else {
            Ok(None) // Still running
        }
    }

    /// Send signal to child process (Unix only)
    ///
    /// Uses the stored PID to send signals directly via libc::kill().
    #[cfg(unix)]
    pub async fn signal(&mut self, sig: i32) -> io::Result<()> {
        let pid = self.child_pid.ok_or_else(|| {
            io::Error::other("Terminal not initialized - no child PID")
        })?;

        // Send signal using libc::kill()
        let result = unsafe { libc::kill(pid as i32, sig) };

        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child_pid {
            let _ = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        }
    }
}

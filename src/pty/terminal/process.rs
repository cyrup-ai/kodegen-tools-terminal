use std::io;
use tokio::time::{Duration, timeout};

use super::types::Terminal;

impl Terminal {
    /// Close the terminal and kill the child process
    ///
    /// This method:
    /// - Checks if the child process has already exited (non-blocking)
    /// - Kills the child process if it's still running
    /// - Signals the writer task to stop by dropping the sender
    /// - Waits for both reader and writer tasks to complete with adaptive timeouts
    ///
    /// For clean shutdown, call this method explicitly before dropping the Terminal.
    /// The Drop implementation provides best-effort cleanup but cannot await.
    ///
    /// # Adaptive Timeouts
    ///
    /// Task wait timeouts are adaptive based on process state:
    /// - **Process already dead**: 100ms timeout (tasks should exit quickly)
    /// - **Process still running**: 5s timeout (allow graceful shutdown)
    ///
    /// This optimization prevents unnecessary waits when stopping already-exited processes.
    ///
    /// # Errors
    ///
    /// Returns error if:
    /// - Reader or writer tasks panicked during execution
    /// - Tasks failed to join properly
    /// - Timeout does NOT cause error (logged only)
    pub async fn close(&mut self) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Drop sender first to signal writer task exit
        // This makes rx.recv() return None, cleanly exiting the writer loop
        drop(self.sender.take());

        // Check if process has already exited BEFORE attempting kill
        let process_already_dead = if self.is_pty_closed() {
            log::debug!("PTY already closed (process exited)");
            true
        } else {
            // Process still running - drop PTY to kill it
            log::debug!("Dropping PTY to kill child process");
            self.pty = None;
            false  // Process was alive (just killed it)
        };

        // Adaptive timeout based on process state
        let task_timeout = if process_already_dead {
            Duration::from_millis(100)  // Dead process = tasks should exit quickly
        } else {
            Duration::from_secs(5)  // Running process = allow graceful shutdown
        };

        // Collect first error but don't return early - must complete ALL cleanup
        let mut first_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;

        // Wait for reader task with adaptive timeout
        if let Some(handle) = self.reader_task.take() {
            match timeout(task_timeout, handle).await {
                Ok(Ok(())) => {
                    log::debug!("Reader task completed successfully");
                }
                Ok(Err(e)) => {
                    log::error!("Reader task panicked or was cancelled: {e:?}");
                    if e.is_panic() {
                        // Collect error but CONTINUE to writer await and cleanup
                        first_error = first_error.or(Some(Box::new(e)));
                    }
                }
                Err(_) => {
                    if process_already_dead {
                        log::warn!(
                            "Reader task timeout after {}ms (process already dead). \
                             This suggests the reader is stuck on blocking I/O.",
                            task_timeout.as_millis()
                        );
                    } else {
                        log::error!(
                            "Reader task timeout after {}s - forcing drop. Task may still be running.",
                            task_timeout.as_secs()
                        );
                    }
                    // Handle dropped, task will be cancelled
                }
            }
        }

        // ALWAYS await writer task (even if reader failed)
        if let Some(handle) = self.writer_task.take() {
            match timeout(task_timeout, handle).await {
                Ok(Ok(())) => {
                    log::debug!("Writer task completed successfully");
                }
                Ok(Err(e)) => {
                    log::error!("Writer task panicked or was cancelled: {e:?}");
                    if e.is_panic() {
                        // Collect error but CONTINUE to cleanup
                        first_error = first_error.or(Some(Box::new(e)));
                    }
                }
                Err(_) => {
                    if process_already_dead {
                        log::warn!(
                            "Writer task timeout after {}ms (process already dead). \
                             This suggests the writer is stuck on blocking I/O.",
                            task_timeout.as_millis()
                        );
                    } else {
                        log::error!(
                            "Writer task timeout after {}s - forcing drop. Task may still be running.",
                            task_timeout.as_secs()
                        );
                    }
                    // Handle dropped, task will be cancelled
                }
            }
        }

        // Return first error AFTER all cleanup complete
        // Note: PTY master is owned by writer task and dropped when task ends
        if let Some(err) = first_error {
            return Err(err);
        }

        Ok(())
    }

    /// Wait for child process to exit and return exit status
    ///
    /// Note: Alacritty's PTY handles child process management internally.
    /// This polls for child exit events.
    pub async fn wait(&mut self) -> io::Result<i32> {
        #[cfg(unix)]
        {
            use alacritty_terminal::tty::{ChildEvent, EventedPty};

            if let Some(pty) = &self.pty {
                let mut pty_guard = pty.lock().await;

                // Poll for child exit event
                loop {
                    if let Some(event) = pty_guard.next_child_event() {
                        match event {
                            ChildEvent::Exited(code) => return Ok(code.unwrap_or(-1)),
                        }
                    }

                    // Small delay to avoid busy-waiting
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            } else {
                Err(io::Error::other("No PTY to wait for"))
            }
        }

        #[cfg(windows)]
        {
            // Windows PTY doesn't expose next_child_event() yet
            // Fall back to checking pty_closed flag
            while !self.is_pty_closed() {
                tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            }
            Ok(0)  // Unknown exit code on Windows
        }
    }

    /// Try to get exit status without waiting (non-blocking)
    pub async fn try_wait(&mut self) -> io::Result<Option<i32>> {
        #[cfg(unix)]
        {
            use alacritty_terminal::tty::{ChildEvent, EventedPty};

            if let Some(pty) = &self.pty {
                let mut pty_guard = pty.lock().await;

                // Check for child exit event (non-blocking)
                if let Some(event) = pty_guard.next_child_event() {
                    match event {
                        ChildEvent::Exited(code) => Ok(Some(code.unwrap_or(-1))),
                    }
                } else {
                    Ok(None)  // Still running
                }
            } else {
                Err(io::Error::other("No PTY to check"))
            }
        }

        #[cfg(windows)]
        {
            // Windows PTY doesn't expose next_child_event() yet
            // Fall back to checking pty_closed flag
            if self.is_pty_closed() {
                Ok(Some(0))  // Unknown exit code on Windows
            } else {
                Ok(None)
            }
        }
    }

    /// Send signal to child process (Unix only)
    ///
    /// Note: Alacritty's PTY doesn't expose direct process ID access.
    /// To kill the process, drop the PTY (self.pty = None).
    #[cfg(unix)]
    pub async fn signal(&mut self, sig: i32) -> io::Result<()> {
        // Alacritty's PTY doesn't expose process ID directly.
        // The standard way to kill is to drop the PTY.
        if sig == 9 || sig == 15 {  // SIGKILL or SIGTERM
            self.pty = None;  // Dropping PTY kills child
            Ok(())
        } else {
            Err(io::Error::other("Only SIGKILL and SIGTERM supported via PTY drop"))
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // If exit_on_close is true, drop PTY to kill the child process
        // Note: We can't await in Drop, so this is best-effort cleanup
        // Users should call close() explicitly for guaranteed cleanup
        if self.config.exit_on_close {
            self.pty = None;  // Dropping PTY kills child
        }
    }
}

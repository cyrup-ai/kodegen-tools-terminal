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
        let process_already_dead = if let Some(child) = &self.child_process {
            let mut child_guard = child.lock().await;
            
            // Try non-blocking check first
            match child_guard.try_wait() {
                Ok(Some(exit_status)) => {
                    log::debug!("Process already exited with status: {exit_status:?}");
                    true  // Process is dead
                }
                Ok(None) => {
                    // Process still running - send SIGKILL
                    if let Err(e) = child_guard.kill() {
                        log::debug!("Failed to kill child process: {e}");
                    } else {
                        log::debug!("Sent SIGKILL to running child process");
                    }
                    false  // Process was alive (just killed it)
                }
                Err(e) => {
                    log::warn!("try_wait failed: {e}");
                    // Attempt kill anyway as fallback
                    let _ = child_guard.kill();
                    false  // Assume alive to be safe (use longer timeout)
                }
            }
        } else {
            true  // No child process = nothing to wait for
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

    /// Wait for child process to exit naturally and return its exit status
    ///
    /// This is a blocking operation that waits until the process exits.
    /// Use this when you want to know the exit code of the process.
    pub async fn wait(&mut self) -> io::Result<portable_pty::ExitStatus> {
        if let Some(child) = &self.child_process {
            let mut child_guard = child.lock().await;
            child_guard.wait()
        } else {
            Err(io::Error::other("No child process to wait for"))
        }
    }

    /// Try to get the exit status without waiting (non-blocking)
    ///
    /// Returns:
    /// - `Ok(Some(status))` if the process has exited
    /// - `Ok(None)` if the process is still running
    /// - `Err(_)` if there's no child process or an error occurred
    pub async fn try_wait(&mut self) -> io::Result<Option<portable_pty::ExitStatus>> {
        if let Some(child) = &self.child_process {
            let mut child_guard = child.lock().await;
            child_guard.try_wait()
        } else {
            Err(io::Error::other("No child process to check"))
        }
    }

    /// Send a signal to the child process (Unix only)
    ///
    /// This allows sending signals like SIGTERM, SIGINT, etc. to the child process.
    /// On non-Unix platforms, this method will return an error.
    #[cfg(unix)]
    pub async fn signal(&mut self, sig: i32) -> io::Result<()> {
        if let Some(child) = &self.child_process {
            let child_guard = child.lock().await;
            // Get the process ID from the child
            if let Some(pid) = child_guard.process_id() {
                // Send signal using libc
                unsafe {
                    if libc::kill(pid as libc::pid_t, sig) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            } else {
                Err(io::Error::other("Failed to get process ID"))
            }
        } else {
            Err(io::Error::other("No child process to signal"))
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // If exit_on_close is true, kill the child process
        // Note: We can't await in Drop, so we spawn a task for best-effort cleanup
        // Users should call close() explicitly for guaranteed cleanup
        if self.config.exit_on_close
            && let Some(child) = &self.child_process
        {
            let child_clone = child.clone();
            // Try to spawn a task to kill the process
            // This may fail if the runtime is shutting down, which is expected
            std::mem::drop(tokio::spawn(async move {
                let mut child_guard = child_clone.lock().await;
                let _ = child_guard.kill();
            }));
        }
    }
}

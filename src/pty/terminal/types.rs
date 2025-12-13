use std::collections::HashMap;

use tokio::sync::broadcast;
use alacritty_terminal::grid::Dimensions;

use crate::validation::{ValidationDecision, CommandManager};
use kodegen_mcp_schema::ToolExecutionContext;

/// Internal result type for terminal command execution
/// Contains both output string (for display) and metadata (for typed output)
#[derive(Debug, Clone)]
pub struct TerminalCommandResult {
    pub terminal: Option<u32>,
    pub output: String,
    pub exit_code: Option<i32>,
    pub cwd: String,
    pub duration_ms: u64,
    pub completed: bool,
    /// List of terminal snapshots (for LIST action)
    pub terminals: Vec<kodegen_mcp_schema::terminal::TerminalSnapshot>,
}

/// Represents a virtual terminal component
/// Terminal emulator using three-thread architecture:
/// - KodegenInteractiveThread: Executes commands, emits ShellOutput events
/// - VteProcessor: Processes VTE sequences, maintains terminal grid, emits TerminalBuffer events
/// - TerminalManager: API layer (subscribes to TerminalBuffer events)
pub struct Terminal {
    /// Terminal ID for this instance
    pub(super) terminal_id: u32,

    /// Handle to KodegenShell thread (drop = shutdown)
    pub(super) shell_handle: Option<crate::shell::ShellHandle>,

    /// JoinHandle for KodegenShell thread
    pub(super) shell_join_handle: Option<tokio::task::JoinHandle<()>>,

    /// Handle to VteProcessor thread (drop = shutdown)
    pub(super) vte_handle: Option<super::vte_processor::VteHandle>,

    /// JoinHandle for VteProcessor thread
    pub(super) vte_join_handle: Option<tokio::task::JoinHandle<()>>,

    /// New validation engine for context-aware command validation
    pub(super) validation_engine: crate::validation::ValidationEngine,
}


impl Terminal {
    /// Create a new terminal builder
    #[must_use]
    pub fn builder() -> super::TerminalBuilder {
        super::TerminalBuilder::new()
    }


    /// Subscribe to terminal buffer updates
    ///
    /// Creates a new subscription to the TerminalBuffer broadcast channel.
    /// Each subscription will receive events sent after the call to subscribe().
    #[must_use]
    pub fn subscribe_buffer(&self) -> Option<broadcast::Receiver<super::TerminalBuffer>> {
        self.vte_handle.as_ref().map(|h| h.buffer_tx.subscribe())
    }

    /// Send cancel signal to stop any currently running command
    ///
    /// Sends cancel signal via channel to KodegenInteractiveThread, which then
    /// cancels the CancellationToken to cleanly stop execution.
    ///
    /// This is safe to call even if no command is running.
    pub async fn send_cancel(&self) -> Result<(), anyhow::Error> {
        let shell_handle = self.shell_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        shell_handle.cancel_tx.send(()).await
            .map_err(|e| anyhow::anyhow!("Failed to send cancel signal: {}", e))?;

        log::info!("Sent cancel signal to KodegenInteractiveThread");
        Ok(())
    }

    /// Send shutdown signals without requiring ownership
    ///
    /// This method is used when Arc::try_unwrap fails but we still need to cleanup resources.
    /// Sends shutdown signals to both KodegenInteractive and VteProcessor threads.
    /// The threads will exit when they receive these signals, cleaning up resources via RAII.
    ///
    /// Note: Cannot await JoinHandles without ownership, so cleanup is asynchronous.
    /// Tokio will automatically cleanup tasks when they exit.
    pub async fn force_shutdown_signals(&self) -> Result<(), anyhow::Error> {
        let shell_handle = self.shell_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // Shutdown KodegenInteractive thread (kills PTY process)
        shell_handle.command_tx.send(crate::pty::terminal::ExecuteCommand::Shutdown).await
            .map_err(|e| anyhow::anyhow!("Failed to send ExecuteCommand::Shutdown: {}", e))?;

        // Shutdown VteProcessor thread (frees VTE grid)
        shell_handle.output_tx.send(crate::pty::terminal::ShellOutput::Shutdown)
            .map_err(|e| anyhow::anyhow!("Failed to send ShellOutput::Shutdown: {}", e))?;

        Ok(())
    }

    /// Execute a command and return output (high-level API)
    ///
    /// Executes command in the persistent shell, collects output filtered by request_id,
    /// and returns TerminalOutput when command completes or times out.
    ///
    /// # Timeout Behavior
    /// - `await_completion_ms = 0`: Fire-and-forget (returns immediately, command runs in background)
    /// - `await_completion_ms > 0`: Wait up to N milliseconds for completion
    ///   - On timeout: returns current terminal output, command continues in background
    ///   - Use action=READ to check progress later
    ///
    /// # Clear Parameter
    /// - `clear`: If true, clears the entire grid (history + viewport + cursor) before executing command
    ///   This ensures read_grid() returns only the new command's output
    ///
    /// # Tail Parameter
    /// - `tail`: Maximum number of lines to return from the end of the buffer
    pub async fn execute_command(
        &self,
        request_id: rmcp::model::RequestId,
        command: String,
        clear: bool,
        await_completion_ms: u64,
        tail: u32,
        ctx: Option<ToolExecutionContext>,
    ) -> Result<TerminalCommandResult, anyhow::Error> {
        use super::TerminalBuffer;
        let start = std::time::Instant::now();

        // NEW: Validate command using ValidationEngine
        let decision = self.validation_engine.validate(&command);

        match decision {
            ValidationDecision::Block { reason, violation_type } => {
                let base_cmd = CommandManager::get_base_command(&command);
                let duration_ms = start.elapsed().as_millis() as u64;

                log::warn!(
                    "Blocked command '{}': {:?} - {}",
                    base_cmd,
                    violation_type,
                    reason
                );

                return Ok(TerminalCommandResult {
                    terminal: Some(self.terminal_id),
                    output: format!(
                        "Error: Command '{}' is not allowed.\n\n\
                         Reason: {}\n\n\
                         Violation type: {:?}\n\n\
                         Please use KODEGEN's MCP tools for safe operations:\n\
                         • fs_read_file / fs_write_file - Safe file operations\n\
                         • fs_search / fs_edit_block - Search and edit files\n\
                         • process_list / process_kill - Process management\n\
                         • terminal - Execute commands in sandboxed shell\n",
                        base_cmd,
                        reason,
                        violation_type
                    ),
                    exit_code: Some(1),
                    cwd: "/".to_string(),
                    duration_ms,
                    completed: true,
                    terminals: Vec::new(),
                });
            }
            ValidationDecision::Allow => {
                // Command allowed, continue to shell execution
            }
        }

        // Get shell handle first (needed for cancel channel and command channel)
        let shell_handle = self.shell_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // Send cancel signal via channel to clear any potentially stuck command
        // The KodegenInteractiveThread will cancel the CancellationToken
        // Use try_send() for immediate failure detection (non-blocking)
        match shell_handle.cancel_tx.try_send(()) {
            Ok(()) => {
                // Cancel signal sent successfully
                log::debug!("Pre-execution cancel signal sent successfully");
            }
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                // Channel full = 4 unprocessed cancel signals = shell is unresponsive
                log::error!("Cancel channel full - shell thread is unresponsive (4 unprocessed signals)");
                return Err(anyhow::anyhow!(
                    "Terminal shell is unresponsive to cancel signals.\n\
                     This indicates the shell thread is stuck or overloaded.\n\
                     Use terminal action=KILL to terminate this terminal, then create a new one."
                ));
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                // Channel closed = shell thread has exited
                log::error!("Cancel channel closed - shell thread has terminated unexpectedly");
                return Err(anyhow::anyhow!(
                    "Terminal shell has terminated unexpectedly.\n\
                     The shell thread is no longer running.\n\
                     Use terminal action=KILL to clean up this terminal, then create a new one."
                ));
            }
        }

        // Clear entire grid if requested (BEFORE sending command)
        if clear
            && let Some(vte_handle) = self.vte_handle.as_ref()
        {
            vte_handle.clear_grid().await?;  // NOW ASYNC
        }

        // Subscribe to buffer events (creates new receiver from broadcast sender)
        let mut buffer_rx = self.subscribe_buffer()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // Send command to KodegenShell executor
        shell_handle.command_tx.send(super::ExecuteCommand::Run {
            request_id: request_id.clone(),
            command: command.clone(),
        }).await?;

        // Special case: await_completion_ms=0 means fire-and-forget background task
        if await_completion_ms == 0 {
            return Ok(TerminalCommandResult {
                terminal: Some(self.terminal_id),
                output: format!(
                    "[Background task started: {}]\n\
                     Command is running in the background.\n\
                     Use action=READ to check progress.",
                    command
                ),
                exit_code: None,
                cwd: "/".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                completed: false,
                terminals: Vec::new(),
            });
        }

        // Wait for final buffer event with matching request_id, with timeout
        let mut final_output = String::new();
        let mut final_cwd = std::path::PathBuf::from("/");
        let mut final_exit_code = None;
        let mut completed = false;

        let timeout_duration = std::time::Duration::from_millis(await_completion_ms);
        let result = tokio::time::timeout(timeout_duration, async {
            loop {
                match buffer_rx.recv().await {
                    Ok(TerminalBuffer::Updated { request_id: event_req_id, lines, cwd, exit_code, is_final, .. }) => {
                        // Filter: only process events matching our request_id
                        if event_req_id == request_id {
                            // Stream last line as progress notification (best-effort, ignore errors)
                            if let Some(ref ctx) = ctx
                                && let Some(last_line) = lines.last()
                                    && !last_line.is_empty() {
                                        let _ = ctx.stream(last_line.clone()).await;
                                    }
                            
                            // Apply tail limit - take last N lines
                            let output_lines = if tail > 0 && lines.len() > tail as usize {
                                &lines[lines.len() - tail as usize..]
                            } else {
                                &lines[..]
                            };
                            final_output = output_lines.join("\r\n");
                            final_cwd = cwd;
                            final_exit_code = Some(exit_code);
                            if is_final {
                                completed = true;
                                break;
                            }
                        }
                    }
                    Ok(_) => continue, // Ignore TitleChanged
                    Err(_) => return Err(anyhow::anyhow!("Buffer channel closed")),
                }
            }
            Ok::<(), anyhow::Error>(())
        }).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        // Handle timeout: return current terminal output
        if result.is_err() {
            final_output.push_str(&format!(
                "\n\n[Command still running after {}ms timeout]\n\
                 [This is the current terminal output]\n\
                 [Command continues in background - use action=READ to check progress]",
                await_completion_ms
            ));

            return Ok(TerminalCommandResult {
                terminal: Some(self.terminal_id),
                output: final_output,
                exit_code: None,
                cwd: final_cwd.display().to_string(),
                duration_ms,
                completed: false,
                terminals: Vec::new(),
            });
        }

        // Command completed successfully within timeout
        Ok(TerminalCommandResult {
            terminal: Some(self.terminal_id),
            output: final_output,
            exit_code: final_exit_code,
            cwd: final_cwd.display().to_string(),
            duration_ms,
            completed,
            terminals: Vec::new(),
        })
    }

    /// Read current terminal state without executing a command
    ///
    /// Returns the current VTE buffer snapshot - useful for checking
    /// progress of long-running commands or background tasks.
    ///
    /// # Tail Parameter
    /// - `tail`: Maximum number of lines to return from the end of the buffer
    pub async fn read_current_state(
        &self,
        terminal_id: u32,
        tail: u32,
    ) -> Result<TerminalCommandResult, anyhow::Error> {
        let start = std::time::Instant::now();

        let vte_handle = self.vte_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // NOW ASYNC - no blocking!
        let snapshot = vte_handle.read_grid(tail).await?;
        let output = snapshot.lines.join("\r\n");

        Ok(TerminalCommandResult {
            terminal: Some(terminal_id),
            output,
            exit_code: snapshot.exit_code,
            cwd: snapshot.cwd,
            duration_ms: start.elapsed().as_millis() as u64,
            completed: true, // READ operation itself is complete
            terminals: Vec::new(),
        })
    }

    /// Explicit async shutdown - waits for all background threads to exit
    ///
    /// This MUST be called before Terminal is dropped to ensure clean shutdown.
    /// Shutdown order:
    /// 1. Send Shutdown to KodegenInteractive, await its exit
    /// 2. Send Shutdown to VteProcessor, await its exit
    pub async fn shutdown(mut self) {
        log::debug!("Terminal {} shutting down", self.terminal_id);

        // Take shell_handle
        let shell_handle = self.shell_handle.take();

        // Send shutdown to KodegenInteractive thread
        if let Some(ref handle) = shell_handle {
            let _ = handle.command_tx.send(crate::pty::terminal::ExecuteCommand::Shutdown).await;
        }

        // Await KodegenInteractive
        if let Some(handle) = self.shell_join_handle.take() {
            tokio::pin!(handle);
            tokio::select! {
                result = &mut handle => {
                    match result {
                        Ok(()) => log::debug!("KodegenInteractiveThread exited cleanly"),
                        Err(e) => log::error!("KodegenInteractiveThread panicked: {}", e),
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                    log::warn!("KodegenInteractiveThread timeout after 2s, aborting task");
                    handle.abort();
                }
            }
        }

        // Send Shutdown to VteProcessor (it subscribes to output_tx)
        if let Some(ref handle) = shell_handle {
            let _ = handle.output_tx.send(crate::pty::terminal::ShellOutput::Shutdown);
        }

        // Drop vte_handle (VteProcessor will exit when shell_output_rx channel closes)
        drop(self.vte_handle.take());

        // Await VteProcessor
        if let Some(handle) = self.vte_join_handle.take() {
            tokio::pin!(handle);
            tokio::select! {
                result = &mut handle => {
                    match result {
                        Ok(()) => log::debug!("VteProcessorThread exited cleanly"),
                        Err(e) => log::error!("VteProcessorThread panicked: {}", e),
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                    log::warn!("VteProcessorThread timeout after 2s, aborting task");
                    handle.abort();
                }
            }
        }

        log::debug!("Terminal {} shutdown complete", self.terminal_id);
    }
}

/// Terminal dimensions and scrollback configuration
///
/// Specifies the visible terminal size (rows × cols) and scrollback buffer capacity.
/// Implements Alacritty's `Dimensions` trait for grid sizing calculations.
///
/// # Fields
/// - `cols`: Number of columns (character width)
/// - `rows`: Number of visible rows (screen height)
/// - `scrollback`: Number of lines retained in scrollback history
#[derive(Debug, Clone, Copy)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
    pub scrollback: usize,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn total_lines(&self) -> usize {
        self.rows as usize + self.scrollback
    }
}

/// Configuration for terminal behavior and shell environment
///
/// Stores terminal initialization parameters including working directory,
/// environment variables, and scrollback capacity. This configuration is
/// retained in the Terminal struct for cloning and debugging purposes.
///
/// # Fields
/// - `cwd`: Optional working directory for the shell
/// - `env_vars`: Environment variables passed to the shell
/// - `shell_path`: Optional custom shell executable path
/// - `scrollback`: Scrollback buffer size (number of lines)
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub cwd: Option<String>,
    pub env_vars: HashMap<String, String>,
    pub shell_path: Option<String>,
    pub scrollback: usize,
}

/// Keyboard key codes for terminal input
///
/// Represents special keys that need escape sequence translation when
/// sent to the terminal. Regular printable characters don't use this enum
/// and are sent directly as UTF-8 bytes via `send_input()`.
///
/// # Usage
/// Use with `Terminal::send_keycode()` to send special keys like arrows,
/// function keys, or control characters that require ANSI escape sequences.
#[derive(Debug, Clone, Copy)]
pub enum KeyCode {
    Backspace,
    Enter,
    Left,
    Right,
    Up,
    Down,
    Tab,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Esc,
    // Add other key codes as needed
}



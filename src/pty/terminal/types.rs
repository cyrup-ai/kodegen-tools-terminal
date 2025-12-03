use std::collections::HashMap;

use tokio::sync::broadcast;
use alacritty_terminal::grid::Dimensions;

use crate::validation::ValidationDecision;
use kodegen_mcp_tool::ToolExecutionContext;

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

    /// Pre-subscribed receiver for TerminalBuffer events (subscribed in builder)
    /// Kept alive to prevent broadcast channel from dropping events before subscribers exist
    #[allow(dead_code)]
    pub(super) buffer_rx: tokio::sync::Mutex<broadcast::Receiver<super::TerminalBuffer>>,

    /// New validation engine for context-aware command validation
    pub(super) validation_engine: crate::validation::ValidationEngine,

    /// Command parsing utilities (kept for parsing, validation moved to ValidationEngine)
    pub(super) command_manager: crate::validation::CommandManager,
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
    /// The Terminal holds an initial subscription (created in builder) to prevent event loss.
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

    /// Execute a command and return output (high-level API)
    ///
    /// Executes command in the persistent shell, collects output filtered by request_id,
    /// and returns TerminalOutput when command completes or times out.
    ///
    /// # Timeout Behavior
    /// - `await_completion_ms = 0`: Fire-and-forget (returns immediately, command runs in background)
    /// - `await_completion_ms > 0`: Wait up to N milliseconds for completion
    ///   - On timeout: returns current 120x200 VTE buffer snapshot, command continues in background
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
                let base_cmd = self.command_manager.get_base_command(&command);
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
        if let Err(e) = shell_handle.cancel_tx.send(()).await {
            log::warn!("Failed to send pre-execution cancel signal: {}", e);
            // Continue anyway - channel may be full or closed
        }

        // Clear entire grid if requested (BEFORE sending command)
        if clear && let Some(vte_handle) = self.vte_handle.as_ref() {
            vte_handle.clear_grid();
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
                            final_output = output_lines.join("\n");
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

        // Handle timeout: return current 120x200 VTE buffer snapshot
        if result.is_err() {
            final_output.push_str(&format!(
                "\n\n[Command still running after {}ms timeout]\n\
                 [This is the current 120x200 VTE buffer snapshot]\n\
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

        // Read grid directly from VteHandle - no broadcast channel
        let vte_handle = self.vte_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        let (lines, cwd, exit_code) = vte_handle.read_grid(tail);
        let output = lines.join("\n");

        Ok(TerminalCommandResult {
            terminal: Some(terminal_id),
            output,
            exit_code,
            cwd,
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



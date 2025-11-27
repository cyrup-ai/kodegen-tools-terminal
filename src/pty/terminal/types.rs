use std::collections::HashMap;

use tokio::sync::broadcast;
use alacritty_terminal::grid::Dimensions;

use crate::validation::ValidationDecision;

/// Represents a virtual terminal component
/// Terminal emulator using three-thread architecture:
/// - BrushExecutor: Executes commands, emits ShellOutput events
/// - VteProcessor: Processes VTE sequences, maintains terminal grid, emits TerminalBuffer events
/// - TerminalManager: API layer (subscribes to TerminalBuffer events)
pub struct Terminal {
    /// Terminal ID for this instance
    pub(super) terminal_id: u32,

    /// Handle to BrushExecutor thread (drop = shutdown)
    pub(super) brush_handle: Option<crate::shell::ShellHandle>,

    /// Handle to VteProcessor thread (drop = shutdown)
    pub(super) vte_handle: Option<super::vte_processor::VteHandle>,

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

    /// Execute a command and return output (high-level API)
    ///
    /// Executes command in the persistent shell, collects output filtered by request_id,
    /// and returns TerminalOutput when command completes or times out.
    ///
    /// # Timeout Behavior
    /// - `await_completion_ms = 0`: Fire-and-forget (returns immediately, command runs in background)
    /// - `await_completion_ms > 0`: Wait up to N milliseconds for completion
    ///   - On timeout: returns current 80x24 VTE buffer snapshot, command continues in background
    ///   - Use action=READ to check progress later
    pub async fn execute_command(
        &self,
        request_id: rmcp::model::RequestId,
        command: String,
        await_completion_ms: u64,
    ) -> Result<kodegen_mcp_schema::terminal::TerminalOutput, anyhow::Error> {
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

                return Ok(kodegen_mcp_schema::terminal::TerminalOutput {
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
                });
            }
            ValidationDecision::Allow => {
                // Command allowed, continue to shell execution
            }
        }

        // Subscribe to buffer events (creates new receiver from broadcast sender)
        let mut buffer_rx = self.subscribe_buffer()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        let brush_handle = self.brush_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // Send command to BrushExecutor
        brush_handle.command_tx.send(super::ExecuteCommand::Run {
            request_id: request_id.clone(),
            command: command.clone(),
        }).await?;

        // Special case: await_completion_ms=0 means fire-and-forget background task
        if await_completion_ms == 0 {
            return Ok(kodegen_mcp_schema::terminal::TerminalOutput {
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
                            final_output = lines.join("\n");
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

        // Handle timeout: return current 80x24 VTE buffer snapshot
        if result.is_err() {
            final_output.push_str(&format!(
                "\n\n[Command still running after {}ms timeout]\n\
                 [This is the current 80x24 VTE buffer snapshot]\n\
                 [Command continues in background - use action=READ to check progress]",
                await_completion_ms
            ));

            return Ok(kodegen_mcp_schema::terminal::TerminalOutput {
                terminal: Some(self.terminal_id),
                output: final_output,
                exit_code: None,
                cwd: final_cwd.display().to_string(),
                duration_ms,
                completed: false,
            });
        }

        // Command completed successfully within timeout
        Ok(kodegen_mcp_schema::terminal::TerminalOutput {
            terminal: Some(self.terminal_id),
            output: final_output,
            exit_code: final_exit_code,
            cwd: final_cwd.display().to_string(),
            duration_ms,
            completed,
        })
    }

    /// Read current terminal state without executing a command
    ///
    /// Returns the current 80x24 VTE buffer snapshot - useful for checking
    /// progress of long-running commands or background tasks.
    pub async fn read_current_state(
        &self,
        terminal_id: u32,
    ) -> Result<kodegen_mcp_schema::terminal::TerminalOutput, anyhow::Error> {
        let start = std::time::Instant::now();

        // Subscribe to buffer to get latest state
        let mut buffer_rx = self.subscribe_buffer()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // Try to get the most recent buffer (non-blocking)
        let (output, cwd) = match buffer_rx.try_recv() {
            Ok(super::TerminalBuffer::Updated { lines, cwd, .. }) => {
                (lines.join("\n"), cwd.display().to_string())
            }
            _ => {
                // No recent buffer available, return empty state
                (String::new(), "/".to_string())
            }
        };

        Ok(kodegen_mcp_schema::terminal::TerminalOutput {
            terminal: Some(terminal_id),
            output,
            exit_code: None, // Unknown if still running
            cwd,
            duration_ms: start.elapsed().as_millis() as u64,
            completed: true, // READ operation itself is complete
        })
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

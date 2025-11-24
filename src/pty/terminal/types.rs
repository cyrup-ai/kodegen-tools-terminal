use std::collections::HashMap;

use tokio::sync::broadcast;
use alacritty_terminal::grid::Dimensions;

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
    /// and returns TerminalOutput when command completes.
    pub async fn execute_command(
        &self,
        request_id: rmcp::model::RequestId,
        command: String,
    ) -> Result<kodegen_mcp_schema::terminal::TerminalOutput, anyhow::Error> {
        use super::TerminalBuffer;
        let start = std::time::Instant::now();

        // Subscribe to buffer events (creates new receiver from broadcast sender)
        let mut buffer_rx = self.subscribe_buffer()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        let brush_handle = self.brush_handle.as_ref()
            .ok_or_else(|| anyhow::anyhow!("Terminal not initialized"))?;

        // Send command to BrushExecutor
        brush_handle.command_tx.send(super::ExecuteCommand::Run {
            request_id: request_id.clone(),
            command,
        }).await?;

        // Wait for final buffer event with matching request_id
        let mut final_output = String::new();
        let mut final_cwd = std::path::PathBuf::from("/");
        let mut final_exit_code;

        loop {
            match buffer_rx.recv().await {
                Ok(TerminalBuffer::Updated { request_id: event_req_id, lines, cwd, exit_code, is_final, .. }) => {
                    // Filter: only process events matching our request_id
                    if event_req_id == request_id {
                        final_output = lines.join("\n");
                        final_cwd = cwd;
                        final_exit_code = exit_code;
                        if is_final {
                            break;
                        }
                    }
                }
                Ok(_) => continue, // Ignore TitleChanged
                Err(_) => return Err(anyhow::anyhow!("Buffer channel closed")),
            }
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        // Build output response
        Ok(kodegen_mcp_schema::terminal::TerminalOutput {
            terminal: self.terminal_id,
            output: final_output,
            exit_code: final_exit_code,
            cwd: final_cwd.display().to_string(),
            duration_ms,
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

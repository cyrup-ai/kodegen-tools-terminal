use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    collections::HashMap,
    thread,
};

use tokio::sync::broadcast;

// Alacritty imports
use alacritty_terminal::term::Term as AlacrittyTerm;
use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Line, Column};

// Import FairMutex for terminal state
use super::sync::FairMutex;

/// No-op event listener for headless terminal usage
///
/// Alacritty's Term sends events for GUI updates (title changes, cursor blinks, etc.).
/// Since we're using it headlessly, we ignore all events.
#[derive(Clone, Copy, Debug)]
pub struct HeadlessEventProxy;

impl EventListener for HeadlessEventProxy {
    fn send_event(&self, _event: TermEvent) {
        // Intentionally empty: we don't need GUI event notifications
        // in a headless terminal server context
    }
}

/// Represents a virtual terminal component
/// Terminal emulator using Alacritty's Term + VTE processor with event loop architecture
pub struct Terminal {
    /// Alacritty's terminal emulator (handles grid, cursor, modes, etc.)
    /// Uses FairMutex for blocking access from event loop thread
    /// Exposed as pub(crate) for manager module grid access
    pub(crate) term: Arc<FairMutex<AlacrittyTerm<HeadlessEventProxy>>>,

    /// Channel sender for writing input to PTY with poller wakeup
    pub(super) sender: Option<super::event_loop::InputSender>,

    /// Terminal size
    pub(super) size: TermSize,

    /// PTY closed flag (set when event loop detects EOF)
    pub(super) pty_closed: Arc<AtomicBool>,

    /// Terminal configuration
    pub(super) config: TerminalConfig,

    /// Single event loop thread (replaces reader + writer + processor tasks)
    /// The event loop takes ownership of the PTY and moves it into the thread
    pub(super) event_loop_thread: Option<thread::JoinHandle<()>>,

    /// Child process ID (captured before moving PTY into event loop)
    /// PID is immutable after process creation, so we capture it once
    pub(super) child_pid: Option<u32>,

    /// Broadcast channel for screen update notifications
    /// Subscribers get notified when screen changes, then call screen() for actual data
    pub(super) output_broadcast: Arc<broadcast::Sender<()>>,
}

impl Clone for Terminal {
    fn clone(&self) -> Self {
        Self {
            term: self.term.clone(),
            sender: self.sender.clone(),
            size: self.size,
            pty_closed: self.pty_closed.clone(),
            config: self.config.clone(),
            event_loop_thread: None,  // Thread handle not cloneable; PTY owned by original instance
            child_pid: self.child_pid,  // u32 is Copy
            output_broadcast: self.output_broadcast.clone(),
        }
    }
}

impl Terminal {
    /// Check if the PTY has been detected as closed by the output reader task
    #[must_use]
    pub fn is_pty_closed(&self) -> bool {
        self.pty_closed.load(Ordering::SeqCst)
    }

    /// Get rendered screen contents as a string
    ///
    /// Renders the current terminal grid to a plain text string.
    #[must_use]
    pub async fn screen(&self) -> Option<String> {
        let term = self.term.clone();
        tokio::task::spawn_blocking(move || {
            let term_guard = term.lock_unfair();
            let grid = term_guard.grid();

            // Pre-allocate: rows * cols + newlines
            let capacity = grid.screen_lines() * (grid.columns() + 1);
            let mut output = String::with_capacity(capacity);

            for line_idx in 0..grid.screen_lines() {
                let line = &grid[Line(line_idx as i32)];

                for col_idx in 0..grid.columns() {
                    let cell = &line[Column(col_idx)];
                    output.push(cell.c);
                }

                // Add newline except for last line
                if line_idx < grid.screen_lines() - 1 {
                    output.push('\n');
                }
            }

            Some(output)
        })
        .await
        .ok()
        .flatten()
    }

    /// Get cell at specific position (useful for debugging/testing)
    #[must_use]
    pub async fn cell_at(&self, row: usize, col: usize) -> Option<char> {
        let term = self.term.clone();
        tokio::task::spawn_blocking(move || {
            let term_guard = term.lock_unfair();
            let grid = term_guard.grid();

            if row >= grid.screen_lines() || col >= grid.columns() {
                return None;
            }

            let cell = &grid[Line(row as i32)][Column(col)];
            Some(cell.c)
        })
        .await
        .ok()
        .flatten()
    }

    /// Get cursor position (row, col)
    #[must_use]
    pub async fn cursor_position(&self) -> (usize, usize) {
        let term = self.term.clone();
        tokio::task::spawn_blocking(move || {
            let term_guard = term.lock_unfair();
            let cursor = term_guard.grid().cursor.point;
            (cursor.line.0 as usize, cursor.column.0)
        })
        .await
        .unwrap_or((0, 0))
    }

    /// Check if alternate screen is active
    #[must_use]
    pub async fn is_alt_screen(&self) -> bool {
        let term = self.term.clone();
        tokio::task::spawn_blocking(move || {
            let term_guard = term.lock_unfair();
            term_guard.mode().contains(alacritty_terminal::term::TermMode::ALT_SCREEN)
        })
        .await
        .unwrap_or(false)
    }

    /// Subscribe to real-time output stream
    ///
    /// Returns a broadcast receiver that receives new output lines as they're processed.
    /// Multiple subscribers can listen concurrently.
    ///
    /// Subscribers receive notifications when screen updates, then call screen() to get actual data.
    /// This follows Alacritty's pattern of lightweight notifications instead of broadcasting data.
    #[must_use]
    pub fn subscribe_output(&self) -> broadcast::Receiver<()> {
        self.output_broadcast.subscribe()
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

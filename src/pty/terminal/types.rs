use std::{
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    collections::HashMap,
};

use bytes::Bytes;
use tokio::{
    sync::{Mutex, mpsc::{Receiver, Sender}},
    task,
};

// Alacritty imports
use parking_lot::RwLock;  // More efficient than std::sync::RwLock
use alacritty_terminal::term::Term as AlacrittyTerm;
use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Line, Column};
use vte::ansi::Processor;

// Use the public Pty type which is re-exported from platform-specific modules
use alacritty_terminal::tty::Pty;

/// No-op event listener for headless terminal usage
///
/// Alacritty's Term sends events for GUI updates (title changes, cursor blinks, etc.).
/// Since we're using it headlessly, we ignore all events.
#[derive(Clone, Copy, Debug)]
pub struct HeadlessEventProxy;  // Already pub, just needs to be exported in mod.rs

impl EventListener for HeadlessEventProxy {
    fn send_event(&self, _event: TermEvent) {
        // Intentionally empty: we don't need GUI event notifications
        // in a headless terminal server context
    }
}

/// Represents a virtual terminal component
/// Terminal emulator using Alacritty's Term + VTE processor
pub struct Terminal {
    /// Alacritty's terminal emulator (handles grid, cursor, modes, etc.)
    pub(crate) term: Arc<RwLock<AlacrittyTerm<HeadlessEventProxy>>>,

    /// VTE processor (processes ANSI escape sequences)
    pub(super) processor: Arc<RwLock<Processor>>,

    /// Channel sender for writing input to PTY
    pub(super) sender: Option<Sender<Bytes>>,

    /// Channel receiver for writing input to PTY (taken by writer task)
    pub(super) receiver: Option<Receiver<Bytes>>,

    /// Terminal size
    pub(super) size: TermSize,

    /// PTY closed flag (set when reader task detects EOF)
    pub(super) pty_closed: Arc<AtomicBool>,

    /// Terminal configuration
    pub(super) config: TerminalConfig,

    /// Alacritty's PTY handle (platform-specific)
    pub(super) pty: Option<Arc<Mutex<Pty>>>,

    /// Reader task handle (processes PTY output)
    pub(super) reader_task: Option<task::JoinHandle<()>>,

    /// Writer task handle (sends input to PTY)
    pub(super) writer_task: Option<task::JoinHandle<()>>,
}

impl Clone for Terminal {
    fn clone(&self) -> Self {
        Self {
            term: self.term.clone(),
            processor: self.processor.clone(),
            sender: self.sender.clone(),
            receiver: None,  // Receiver cannot be cloned
            size: self.size,
            pty_closed: self.pty_closed.clone(),
            config: self.config.clone(),
            pty: self.pty.clone(),
            reader_task: None,  // Task handles cannot be cloned
            writer_task: None,
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
    /// This replaces vt100::Screen::contents().
    #[must_use]
    pub fn screen(&self) -> Option<String> {
        let term = self.term.read();
        let grid = term.grid();

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
    }

    /// Get cell at specific position (useful for debugging/testing)
    #[must_use]
    pub fn cell_at(&self, row: usize, col: usize) -> Option<char> {
        let term = self.term.read();
        let grid = term.grid();

        if row >= grid.screen_lines() || col >= grid.columns() {
            return None;
        }

        let cell = &grid[Line(row as i32)][Column(col)];
        Some(cell.c)
    }

    /// Get cursor position (row, col)
    #[must_use]
    pub fn cursor_position(&self) -> (usize, usize) {
        let term = self.term.read();
        let cursor = term.grid().cursor.point;
        (cursor.line.0 as usize, cursor.column.0)
    }

    /// Check if alternate screen is active
    #[must_use]
    pub fn is_alt_screen(&self) -> bool {
        let term = self.term.read();
        term.mode().contains(alacritty_terminal::term::TermMode::ALT_SCREEN)
    }
}

/// Size information for the terminal
#[derive(Debug, Clone, Copy)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl Dimensions for TermSize {
    fn columns(&self) -> usize {
        self.cols as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows as usize
    }

    fn total_lines(&self) -> usize {
        // For now, match screen_lines (can add scrollback later via config)
        self.rows as usize
    }
}

/// Terminal color mode options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorMode {
    Basic,    // No colors
    Color,    // Basic 8 colors
    Color256, // 256 colors
    #[default]
    TrueColor, // 24-bit true color
}

/// Terminal bell style options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BellStyle {
    None, // No bell
    #[default]
    Visual, // Visual bell
    Audible, // Audible bell
}

/// Configuration for terminal behavior and appearance
#[derive(Debug, Clone)]
pub struct TerminalConfig {
    pub command: Option<String>,
    pub cwd: Option<String>,
    pub env_vars: HashMap<String, String>,
    pub shell: bool,
    pub shell_path: Option<String>,
    pub colors: ColorMode,
    pub scrollback: usize,
    pub cursor_blink: bool,
    pub application_cursor_keys: bool,
    pub bracketed_paste: bool,
    pub mouse_reporting: bool,
    pub alternate_screen: bool,
    pub exit_on_close: bool,
    pub bell_style: BellStyle,
}

/// Keyboard key codes for terminal input
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

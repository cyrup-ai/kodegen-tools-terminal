// Public modules
pub mod terminal;

// Re-export tokio for async runtime
pub use tokio;

// Core terminal types
pub use terminal::{
    BellStyle, ColorMode, KeyCode, TermSize, Terminal, TerminalBuilder,
    HeadlessEventProxy,  // NEW: Export event proxy
};

// Alacritty re-exports (replace portable_pty::CommandBuilder)
pub use alacritty_terminal::tty::{Options as PtyOptions, Shell};

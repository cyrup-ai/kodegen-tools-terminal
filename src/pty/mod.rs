// Public modules
pub mod cwd;
pub mod terminal;

// Re-export tokio for async runtime
pub use tokio;

// Core terminal types
pub use terminal::{
    KeyCode, TermSize, Terminal, TerminalBuilder,
    HeadlessEventProxy,
};

// Alacritty re-exports (replace portable_pty::CommandBuilder)
pub use alacritty_terminal::tty::{Options as PtyOptions, Shell};

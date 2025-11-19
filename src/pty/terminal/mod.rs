// Core types and structures
mod types;
pub use types::{BellStyle, ColorMode, KeyCode, TermSize, Terminal, TerminalConfig, HeadlessEventProxy};

// Builder pattern
mod builder;
pub use builder::TerminalBuilder;

// Factory methods
mod factory;

// PTY initialization
mod initialization;

// Command execution and input operations
mod execution;

// Process management
mod process;

// Shell detection utilities
mod shell;

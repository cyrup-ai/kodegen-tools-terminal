// Core types and structures
mod types;
pub use types::{KeyCode, TermSize, Terminal, TerminalConfig, HeadlessEventProxy};

// Synchronization primitives (FairMutex)
pub mod sync;

// Event loop implementation (uses generic EventedPty trait from Alacritty)
mod event_loop;

// Builder pattern
mod builder;
pub use builder::TerminalBuilder;

// Factory methods
mod factory;

// Command execution and input operations
mod execution;

// Process management
mod process;

// Shell detection utilities
mod shell;

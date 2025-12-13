//! Terminal module - Three-thread architecture
//!
//! # Architecture
//!
//! - **BrushExecutor**: Executes commands, emits ShellOutput events
//! - **VteProcessor**: Processes VTE sequences, maintains terminal grid, emits TerminalBuffer events
//! - **TerminalManager**: API layer (subscribes to TerminalBuffer events)

pub mod types;
pub use types::{KeyCode, TermSize, Terminal, TerminalConfig, TerminalCommandResult};

mod events;
pub use events::{ShellOutput, TerminalBuffer, ExecuteCommand};

mod event_bridge;
pub(super) use event_bridge::EventBridge;

mod vte_processor;
pub use vte_processor::{VteProcessorThread, VteHandle};

mod builder;
pub use builder::TerminalBuilder;

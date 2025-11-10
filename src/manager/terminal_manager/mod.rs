//! Terminal manager module
//!
//! This module provides the `TerminalManager` for managing PTY-based terminal sessions.
//! The implementation is organized into logical submodules:
//!
//! - `constants`: Configuration constants (cleanup intervals, buffer limits, session limits)
//! - `types`: Data structures (session info, metrics, response types)
//! - `repl_detection`: REPL prompt detection utilities
//! - `session_lifecycle`: Spawning and executing commands
//! - `session_io`: Reading output and sending input
//! - `session_control`: Terminating sessions and checking existence
//! - `session_queries`: Listing sessions and retrieving metrics
//! - `cleanup`: Session cleanup and background task management

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use tokio::sync::Mutex;

// Submodule declarations
mod cleanup;
mod constants;
mod repl_detection;
mod session_control;
mod session_io;
mod session_lifecycle;
mod session_queries;
mod types;

// Re-export public types
pub use types::{
    ActiveTerminalSession, CompletedTerminalSession, TerminalCommandResult, TerminalMetrics,
    TerminalOutputResponse, TerminalSessionInfo,
};

// ============================================================================
// TERMINAL MANAGER
// ============================================================================

/// Terminal manager for handling command execution and session management
///
/// The `TerminalManager` provides PTY-based terminal sessions with VT100 emulation.
/// It manages session lifecycle, I/O operations, and automatic cleanup.
///
/// # Architecture
///
/// The manager maintains two collections:
/// - `sessions`: Active sessions being tracked
/// - `completed_sessions`: Recently completed sessions (retained briefly for querying)
///
/// Sessions are identified by synthetic PIDs (not OS PIDs) starting from 1000.
///
/// # Implementation Methods
///
/// Methods are implemented across multiple submodules:
/// - `spawn_command`, `execute_command` → `session_lifecycle.rs`
/// - `get_output`, `send_input` → `session_io.rs`
/// - `force_terminate`, `get_session` → `session_control.rs`
/// - `list_active_sessions`, `list_completed_sessions`, `metrics` → `session_queries.rs`
/// - `cleanup_sessions`, `start_cleanup_task` → `cleanup.rs`
#[derive(Clone)]
pub struct TerminalManager {
    sessions: Arc<Mutex<HashMap<u32, TerminalSessionInfo>>>,
    completed_sessions: Arc<Mutex<HashMap<u32, CompletedTerminalSession>>>,
    next_pid: Arc<AtomicU32>,
}

impl TerminalManager {
    /// Create a new terminal manager instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            completed_sessions: Arc::new(Mutex::new(HashMap::new())),
            next_pid: Arc::new(AtomicU32::new(1000)),
        }
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

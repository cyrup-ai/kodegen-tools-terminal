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
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

// Submodule declarations
mod cleanup;
mod constants;
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
/// - `sessions`: Active sessions being tracked (keyed by (session_id, terminal_id))
/// - `completed_sessions`: Recently completed sessions (retained briefly for querying)
///
/// Sessions are identified by a composite key: (session_id, terminal_id)
/// - `session_id`: String identifier for the session (e.g., "default", "user-session-1")
/// - `terminal_id`: Numeric identifier starting from 1000
///
/// # Implementation Methods
///
/// Methods are implemented across multiple submodules:
/// - `spawn_command` → `session_lifecycle.rs`
/// - `get_output`, `send_input` → `session_io.rs`
/// - `force_terminate`, `get_session` → `session_control.rs`
/// - `list_active_sessions`, `list_completed_sessions`, `metrics` → `session_queries.rs`
/// - `cleanup_sessions`, `start_cleanup_task` → `cleanup.rs`
#[derive(Clone)]
pub struct TerminalManager {
    sessions: Arc<Mutex<HashMap<(String, u32), TerminalSessionInfo>>>,
    completed_sessions: Arc<Mutex<HashMap<(String, u32), CompletedTerminalSession>>>,
}

impl TerminalManager {
    /// Create a new terminal manager instance
    #[must_use]
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            completed_sessions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get the current working directory of a terminal
    ///
    /// Queries the PTY child process's CWD using OS-specific APIs.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    ///
    /// # Returns
    /// The current working directory, or None if terminal not found or CWD unavailable
    pub async fn get_terminal_cwd(&self, connection_id: &str, terminal_id: u32) -> Option<PathBuf> {
        let key = (connection_id.to_string(), terminal_id);
        let sessions = self.sessions.lock().await;
        if let Some(info) = sessions.get(&key) {
            let terminal = info.terminal.read().await;
            if let Some(pid) = terminal.try_get_pid() {
                return crate::pty::cwd::get_cwd(pid).ok();
            }
        }
        None
    }
}

impl Default for TerminalManager {
    fn default() -> Self {
        Self::new()
    }
}

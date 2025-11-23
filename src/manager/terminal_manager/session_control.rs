//! Session control operations
//!
//! This module handles control operations for terminal sessions:
//! - Force termination of running sessions
//! - Session existence checking

use chrono::Utc;
use kodegen_mcp_tool::error::McpError;

impl super::TerminalManager {
    /// Force terminate a running command
    ///
    /// Kills the PTY child process and waits for cleanup.
    /// Uses `terminal.close()` which handles graceful SIGTERM → SIGKILL escalation.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number to terminate
    ///
    /// # Errors
    /// - `McpError::InvalidArguments`: Session not found
    /// - `McpError::Other`: Terminal close failed
    pub async fn force_terminate(&self, connection_id: &str, terminal_id: u32) -> Result<(), McpError> {
        // 1. Get session using composite key
        let sessions = self.sessions.lock().await;
        let key = (connection_id.to_string(), terminal_id);
        let session = sessions
            .get(&key)
            .ok_or_else(|| {
                McpError::InvalidArguments(format!(
                    "No active terminal found: connection_id={}, terminal_id={}",
                    connection_id,
                    terminal_id
                ))
            })?
            .clone();
        drop(sessions);

        // 2. Close terminal (kills child, waits for tasks)
        let mut terminal = session.terminal.write().await;
        terminal.close().await.map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Failed to close terminal: connection_id={}, terminal_id={}, error={}",
                connection_id,
                terminal_id,
                e
            ))
        })?;

        log::warn!(
            "Terminal terminated: connection_id={}, terminal_id={}, runtime={}s",
            connection_id,
            terminal_id,
            (Utc::now() - session.start_time).num_seconds()
        );
        Ok(())
    }

    /// Get a session by connection_id and terminal_id, returns the terminal_id if session exists
    pub async fn get_session(&self, connection_id: &str, terminal_id: u32) -> Option<u32> {
        let key = (connection_id.to_string(), terminal_id);
        let sessions_guard = self.sessions.lock().await;
        if sessions_guard.contains_key(&key) {
            Some(terminal_id)
        } else {
            None
        }
    }
}

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
    /// - `pid`: Process ID to terminate
    ///
    /// # Errors
    /// - `McpError::InvalidArguments`: Session not found
    /// - `McpError::Other`: Terminal close failed
    pub async fn force_terminate(&self, pid: u32) -> Result<(), McpError> {
        // 1. Get session
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(&pid)
            .ok_or_else(|| {
                McpError::InvalidArguments(format!("No active session found for PID: {pid}"))
            })?
            .clone();
        drop(sessions);

        // 2. Close terminal (kills child, waits for tasks)
        let mut terminal = session.terminal.write().await;
        terminal.close().await.map_err(|e| {
            McpError::Other(anyhow::anyhow!(
                "Failed to close terminal for PID {pid}: {e}"
            ))
        })?;

        log::warn!(
            "Session terminated: pid={}, runtime={}s",
            pid,
            (Utc::now() - session.start_time).num_seconds()
        );
        Ok(())
    }

    /// Get a session by PID, returns the PID if session exists
    pub async fn get_session(&self, pid: u32) -> Option<u32> {
        let sessions_guard = self.sessions.lock().await;
        if sessions_guard.contains_key(&pid) {
            Some(pid)
        } else {
            None
        }
    }
}

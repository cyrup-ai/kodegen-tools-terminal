//! Session query operations
//!
//! This module handles querying session information:
//! - Listing active sessions
//! - Listing completed sessions
//! - Retrieving metrics and statistics

use super::types::{ActiveTerminalSession, CompletedTerminalSession, TerminalMetrics};
use chrono::Utc;

impl super::TerminalManager {
    /// List all active terminal sessions, optionally filtered by connection_id
    ///
    /// # Parameters
    /// - `connection_id`: Optional filter for specific connection (None = all connections)
    pub async fn list_active_sessions(&self, connection_id: Option<&str>) -> Vec<ActiveTerminalSession> {
        let sessions_guard = self.sessions.lock().await;
        let now = Utc::now();

        let mut results = Vec::new();
        for ((conn_id, tid), session) in sessions_guard.iter() {
            if connection_id.map(|id| id == conn_id.as_str()).unwrap_or(true) {
                // Convert runtime to milliseconds, clamping negative to 0
                let runtime_ms = (now - session.start_time).num_milliseconds();
                let runtime = u64::try_from(runtime_ms).unwrap_or(0);

                // Get CWD using async read
                let cwd = {
                    let term = session.terminal.read().await;
                    if let Some(pid) = term.try_get_pid() {
                        crate::pty::cwd::get_cwd(pid).ok()
                    } else {
                        None
                    }
                };

                results.push(ActiveTerminalSession {
                    connection_id: conn_id.clone(),
                    terminal_id: *tid,
                    command: session.command.clone(),
                    still_running: session.still_running,
                    runtime,
                    cwd,
                });
            }
        }
        results
    }

    /// List all completed terminal sessions, optionally filtered by connection_id
    ///
    /// # Parameters
    /// - `connection_id`: Optional filter for specific connection (None = all connections)
    pub async fn list_completed_sessions(&self, connection_id: Option<&str>) -> Vec<CompletedTerminalSession> {
        let completed_guard = self.completed_sessions.lock().await;
        completed_guard
            .iter()
            .filter(|((conn_id, _tid), _session)| {
                connection_id.map(|id| id == conn_id.as_str()).unwrap_or(true)
            })
            .map(|((_conn_id, _tid), session)| session.clone())
            .collect()
    }

    /// Get metrics for monitoring terminal session health
    ///
    /// Returns statistics about session usage, helping identify:
    /// - Memory leaks (`active_sessions` growing)
    /// - Cleanup issues (`completed_sessions` not being cleared)
    /// - Performance problems (average duration increasing)
    pub async fn metrics(&self) -> TerminalMetrics {
        let sessions = self.sessions.lock().await;
        let completed = self.completed_sessions.lock().await;

        // Calculate average session duration from completed sessions
        let mut total_duration_secs = 0.0;
        let mut count = 0;

        for session in completed.values() {
            if let Ok(duration) = session.end_time.duration_since(session.start_time) {
                total_duration_secs += duration.as_secs_f64();
                count += 1;
            }
        }

        let average_duration = if count > 0 {
            total_duration_secs / f64::from(count)
        } else {
            0.0
        };

        // Calculate total sessions created from current + completed counts
        let total_created = sessions.len() as u64 + completed.len() as u64;

        TerminalMetrics {
            total_sessions_created: total_created,
            active_sessions: sessions.len(),
            completed_sessions: completed.len(),
            average_session_duration_secs: average_duration,
            max_concurrent_sessions: sessions.len(), // Current as proxy (could track separately)
            total_commands_executed: total_created,
        }
    }
}

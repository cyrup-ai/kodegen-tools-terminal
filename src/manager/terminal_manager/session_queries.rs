//! Session query operations
//!
//! This module handles querying session information:
//! - Listing active sessions
//! - Listing completed sessions
//! - Retrieving metrics and statistics

use super::types::{ActiveTerminalSession, CompletedTerminalSession, TerminalMetrics};
use chrono::Utc;
use std::sync::atomic::Ordering as AtomicOrdering;

impl super::TerminalManager {
    /// List all active terminal sessions
    #[must_use]
    pub fn list_active_sessions(&self) -> Vec<ActiveTerminalSession> {
        if let Ok(sessions_guard) = self.sessions.try_lock() {
            let now = Utc::now();
            sessions_guard
                .values()
                .map(|session| {
                    // Convert runtime to milliseconds, clamping negative to 0
                    let runtime_ms = (now - session.start_time).num_milliseconds();
                    let runtime = u64::try_from(runtime_ms).unwrap_or(0);
                    ActiveTerminalSession {
                        pid: session.pid,
                        is_blocked: session.is_blocked,
                        runtime,
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    }

    /// List all completed terminal sessions
    #[must_use]
    pub fn list_completed_sessions(&self) -> Vec<CompletedTerminalSession> {
        if let Ok(completed_guard) = self.completed_sessions.try_lock() {
            completed_guard.values().cloned().collect()
        } else {
            Vec::new()
        }
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

        // Get total sessions created from atomic counter
        let total_created = u64::from(self.next_pid.load(AtomicOrdering::SeqCst)) - 1000;

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

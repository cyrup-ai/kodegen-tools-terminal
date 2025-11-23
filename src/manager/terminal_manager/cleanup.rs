//! Session cleanup operations
//!
//! This module handles cleanup of old terminal sessions to prevent unbounded memory growth.
//! It implements a two-tier cleanup strategy with differentiated retention periods.

use super::constants::{
    ACTIVE_SESSION_RETENTION_SECS, CLEANUP_INTERVAL_SECS, COMPLETED_SESSION_RETENTION_SECS,
};
use super::types::{CompletedTerminalSession, TerminalSessionInfo};
use chrono::Utc;
use std::sync::Arc;
use std::time::{Duration, Instant};

impl super::TerminalManager {
    /// Clean up old completed sessions with differentiated retention.
    ///
    /// Moves completed sessions to `completed_sessions` `HashMap` before removing.
    /// Explicitly closes terminals to prevent resource leaks.
    /// Retention policy:
    /// - Completed sessions: 30 seconds retention in active, then 5 minutes in completed
    /// - Active sessions: 5 minutes retention
    pub async fn cleanup_sessions(&self) {
        let now = Instant::now();

        // Calculate different cutoff times
        let active_cutoff = now
            .checked_sub(Duration::from_secs(ACTIVE_SESSION_RETENTION_SECS))
            .unwrap_or(now);

        let completed_cutoff = now
            .checked_sub(Duration::from_secs(COMPLETED_SESSION_RETENTION_SECS))
            .unwrap_or(now);

        let mut sessions = self.sessions.lock().await;
        let initial_count = sessions.len();

        // Collect sessions to remove - can't call async close() inside retain()
        let mut to_remove: Vec<((String, u32), TerminalSessionInfo)> = Vec::new();
        let mut to_complete: Vec<((String, u32), TerminalSessionInfo)> = Vec::new();

        for ((connection_id, terminal_id), session) in sessions.iter() {
            // Check completion status from terminal
            let is_complete = {
                let terminal = session.terminal.read().await;
                terminal.is_pty_closed()
            };

            let last_read = *session.last_read_time.read().await;

            // Differentiated retention based on completion status
            let should_keep = if is_complete {
                // Completed sessions: shorter retention (30 seconds)
                last_read > completed_cutoff
            } else {
                // Active sessions: longer retention (5 minutes)
                last_read > active_cutoff
            };

            if !should_keep {
                let key = (connection_id.clone(), *terminal_id);
                if is_complete {
                    to_complete.push((key, session.clone()));
                } else {
                    to_remove.push((key, session.clone()));
                }
            }
        }

        // Close terminals BEFORE removing from HashMap (prevents resource leaks)
        for (key, session) in &to_remove {
            log::debug!("Closing terminal for inactive session: connection_id={}, terminal_id={}", 
                       key.0, key.1);
            let mut terminal = session.terminal.write().await;
            if let Err(e) = terminal.close().await {
                log::error!("Failed to close terminal: connection_id={}, terminal_id={}, error={}", 
                           key.0, key.1, e);
            }
        }

        // Close terminals for completed sessions too
        for (key, session) in &to_complete {
            log::debug!("Closing terminal for completed session: connection_id={}, terminal_id={}", 
                       key.0, key.1);
            let mut terminal = session.terminal.write().await;
            if let Err(e) = terminal.close().await {
                log::error!("Failed to close terminal: connection_id={}, terminal_id={}, error={}", 
                           key.0, key.1, e);
            }
        }

        // Now safe to remove from HashMap
        for (key, _) in to_remove {
            sessions.remove(&key);
        }

        // Move completed sessions to completed_sessions HashMap
        drop(sessions); // Release lock before acquiring completed_sessions lock

        if !to_complete.is_empty() {
            let moved_count = to_complete.len();
            let mut completed = self.completed_sessions.lock().await;

            // Remove from active sessions first
            let mut sessions = self.sessions.lock().await;
            for (key, session) in to_complete {
                let (connection_id, terminal_id) = key.clone();
                sessions.remove(&key);

                // Get final exit code if available
                let exit_code = {
                    let mut terminal = session.terminal.write().await;
                    terminal
                        .try_wait()
                        .await
                        .ok()
                        .flatten()
                };

                // Get final output from Alacritty Grid
                let output = {
                    let terminal = session.terminal.read().await;
                    terminal.screen().await.unwrap_or_default()
                };

                // Convert timestamps (session.start_time is DateTime<Utc>, need SystemTime)
                let start_time = {
                    let chrono_duration: chrono::Duration = Utc::now() - session.start_time;
                    if let Ok(std_duration) = chrono_duration.to_std() {
                        std::time::SystemTime::now()
                            .checked_sub(std_duration)
                            .unwrap_or_else(std::time::SystemTime::now)
                    } else {
                        // Negative duration (clock skew): start_time is in future, use current time
                        std::time::SystemTime::now()
                    }
                };

                let end_time = std::time::SystemTime::now();

                // Create completed session record
                let completed_session = CompletedTerminalSession {
                    connection_id,
                    terminal_id,
                    output,
                    exit_code,
                    start_time,
                    end_time,
                };

                completed.insert(key, completed_session);
            }

            log::info!("Moved {moved_count} sessions to completed_sessions");
        }

        let cleaned_count = initial_count - self.sessions.lock().await.len();
        if cleaned_count > 0 {
            log::info!(
                "Session cleanup: removed={}, active={}, completed={}",
                cleaned_count,
                self.sessions.lock().await.len(),
                self.completed_sessions.lock().await.len()
            );
        }
    }

    /// Clean up old completed sessions (older than 5 minutes)
    ///
    /// Called periodically by cleanup task to prevent unbounded memory growth.
    async fn cleanup_completed_sessions(&self) {
        let now = std::time::SystemTime::now();
        let cutoff = Duration::from_secs(5 * 60); // 5 minutes

        let mut completed = self.completed_sessions.lock().await;
        let initial_count = completed.len();

        completed.retain(|(connection_id, terminal_id), session| {
            let age = now
                .duration_since(session.end_time)
                .unwrap_or(Duration::ZERO);
            let should_keep = age < cutoff;

            if !should_keep {
                log::debug!(
                    "Removing old completed session: connection_id={}, terminal_id={} (age: {:?})",
                    connection_id, terminal_id, age
                );
            }

            should_keep
        });

        let removed_count = initial_count - completed.len();
        if removed_count > 0 {
            log::info!("Cleaned up {removed_count} old completed sessions");
        }
    }

    /// Start background cleanup task (call once at server startup).
    ///
    /// Spawns a tokio task that runs cleanup every minute with differentiated retention:
    /// - Active sessions: 5 minutes retention
    /// - Completed sessions: 30 seconds retention
    ///
    /// # Pattern
    /// Follows the same pattern as `sequential_thinking` cleanup:
    /// packages/sequential-thinking/src/sequential_thinking.rs:353-363
    ///
    /// # Usage
    /// Called from main.rs after wrapping manager in Arc:
    /// ```rust,no_run
    /// use kodegen_tools_terminal::manager::TerminalManager;
    /// use std::sync::Arc;
    ///
    /// let terminal_manager = TerminalManager::new();
    /// let terminal_manager_arc = Arc::new(terminal_manager.clone());
    /// terminal_manager_arc.start_cleanup_task();
    /// ```
    pub fn start_cleanup_task(self: Arc<Self>) {
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(CLEANUP_INTERVAL_SECS));
            loop {
                interval.tick().await;

                // Clean up both active and completed sessions
                self.cleanup_sessions().await;
                self.cleanup_completed_sessions().await;
            }
        });
    }
}

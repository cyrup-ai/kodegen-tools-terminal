//! Session lifecycle management
//!
//! This module handles spawning and executing terminal commands.
//! It manages the creation of PTY-based terminal sessions.

use super::constants::{MAX_OUTPUT_BUFFER_LINES, MAX_SESSIONS};
use super::types::TerminalSessionInfo;
use crate::pty::Terminal;
use chrono::Utc;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

impl super::TerminalManager {
    /// Spawn a new interactive PTY terminal
    ///
    /// This creates a PTY-based terminal with VT100 emulation, enabling:
    /// - Interactive programs (vim, less, top)
    /// - ANSI color sequences
    /// - Proper TTY detection by child processes
    /// - Persistent shell sessions for multiple commands
    ///
    /// After spawning, use send_input() to execute commands in the shell.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    /// - `shell_path`: Optional shell path override
    ///
    /// # Returns
    /// Ok(()) if successful
    pub async fn spawn_command(
        &self,
        connection_id: &str,
        terminal_id: u32,
        shell_path: Option<&str>,
    ) -> Result<(), anyhow::Error> {
        // 1. Build PTY terminal OUTSIDE lock (slow operation)
        let mut builder = Terminal::builder()
            .size(24, 80)
            .scrollback(MAX_OUTPUT_BUFFER_LINES);

        if let Some(shell) = shell_path {
            builder = builder.shell_path(shell);
        }

        // 2. Build and initialize PTY OUTSIDE lock (spawns interactive shell)
        let terminal = builder.build().await?;

        // 3. Create session info
        let session = TerminalSessionInfo {
            connection_id: connection_id.to_string(),
            terminal_id,
            command: String::new(), // No initial command - shell is interactive
            terminal: Arc::new(RwLock::new(terminal)),
            last_read_time: Arc::new(RwLock::new(Instant::now())),
            still_running: false,
            ready_for_input: true, // Shell is ready for input immediately
            start_time: Utc::now(),
        };

        // 4. ATOMIC: Check limit and insert in SINGLE lock scope
        let mut sessions = self.sessions.lock().await;
        if sessions.len() >= MAX_SESSIONS {
            // Must clean up spawned terminal before returning error
            drop(sessions); // Release lock before async close

            let mut terminal = session.terminal.write().await;
            if let Err(e) = terminal.close().await {
                log::error!("Failed to clean up terminal after MAX_SESSIONS reached: {e}");
            }

            return Err(anyhow::anyhow!(
                "Maximum session limit reached ({MAX_SESSIONS}/{MAX_SESSIONS} sessions). \
                 Please wait for existing sessions to complete or stop them manually."
            ));
        }

        // Insert with composite key
        let key = (connection_id.to_string(), terminal_id);
        sessions.insert(key, session);
        log::info!(
            "Interactive shell spawned: connection_id={}, terminal_id={}, active={}/{}",
            connection_id,
            terminal_id,
            sessions.len(),
            MAX_SESSIONS
        );

        Ok(())
    }
}

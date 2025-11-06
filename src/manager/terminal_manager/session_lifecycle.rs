//! Session lifecycle management
//!
//! This module handles spawning and executing terminal commands.
//! It manages the creation of PTY-based terminal sessions.

use super::constants::{MAX_OUTPUT_BUFFER_LINES, MAX_SESSIONS};
use super::repl_detection::detect_repl_ready;
use super::types::{TerminalCommandResult, TerminalSessionInfo};
use crate::pty::Terminal;
use chrono::Utc;
use std::sync::Arc;
use std::sync::atomic::Ordering as AtomicOrdering;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::time::sleep;

impl super::TerminalManager {
    /// Spawn a new command in a PTY terminal
    ///
    /// This creates a PTY-based terminal with VT100 emulation, enabling:
    /// - Interactive programs (vim, less, top)
    /// - ANSI color sequences
    /// - Proper TTY detection by child processes
    pub async fn spawn_command(
        &self,
        command: &str,
        shell_path: Option<&str>,
    ) -> Result<u32, anyhow::Error> {
        // 1. Build PTY terminal OUTSIDE lock (slow operation)
        let mut builder = Terminal::builder()
            .command(command)
            .size(24, 80)
            .scrollback(MAX_OUTPUT_BUFFER_LINES)
            .shell(true);

        if let Some(shell) = shell_path {
            builder = builder.shell_path(shell);
        }

        let mut terminal = builder.build();

        // 2. Initialize PTY OUTSIDE lock (spawns child process and I/O tasks)
        terminal.init().await?;

        // 3. Generate PID (atomic operation, safe outside lock)
        let pid = self.next_pid.fetch_add(1, AtomicOrdering::SeqCst);

        // 4. Create session info
        let session = TerminalSessionInfo {
            pid,
            command: command.to_string(),
            terminal: Arc::new(RwLock::new(terminal)),
            last_read_time: Arc::new(RwLock::new(Instant::now())),
            is_blocked: false,
            ready_for_input: false,
            start_time: Utc::now(),
        };

        // 5. ATOMIC: Check limit and insert in SINGLE lock scope
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

        // Insert happens in same lock scope as check - ATOMIC
        sessions.insert(pid, session);
        log::info!(
            "Session spawned: pid={}, command={}, active={}/{}",
            pid,
            command,
            sessions.len(),
            MAX_SESSIONS
        );

        Ok(pid)
    }

    /// Execute a command in a new terminal session
    ///
    /// # Errors
    /// Returns error if command execution fails, process cannot be spawned, or I/O errors occur
    pub async fn execute_command(
        &self,
        command: &str,
        initial_delay_ms: Option<u64>,
        shell: Option<&str>,
    ) -> Result<TerminalCommandResult, anyhow::Error> {
        // Use spawn_command which creates PTY terminal
        let pid = self.spawn_command(command, shell).await?;

        // Wait for initial delay to capture quick output (pwd, echo, etc.)
        let delay = Duration::from_millis(initial_delay_ms.unwrap_or(100));
        sleep(delay).await;

        // Get initial output if available
        let output_response = self.get_output(pid, 0, 1000).await;

        let (output, is_blocked, ready_for_input) = if let Some(resp) = output_response {
            let text = resp.lines.join("");
            let ready = detect_repl_ready(&text);
            (text, !ready, ready)
        } else {
            (String::new(), false, false)
        };

        Ok(TerminalCommandResult {
            pid,
            output,
            is_blocked,
            ready_for_input,
        })
    }
}

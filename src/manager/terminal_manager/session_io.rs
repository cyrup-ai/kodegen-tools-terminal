//! Session I/O operations
//!
//! This module handles input/output operations for terminal sessions:
//! - Reading paginated output from VT100 screen buffer
//! - Sending interactive input to PTY

use super::types::TerminalOutputResponse;
use bytes::Bytes;
use std::time::Instant;

impl super::TerminalManager {
    /// Get paginated output from a running command
    ///
    /// Extracts text from the VT100 screen buffer with pagination support.
    ///
    /// # Parameters
    /// - `pid`: Process ID
    /// - `offset`: Starting line (negative = tail from end)
    /// - `length`: Maximum lines to return
    ///
    /// # Returns
    /// Terminal output with pagination info, or None if session not found
    pub async fn get_output(
        &self,
        pid: u32,
        offset: i64,
        length: usize,
    ) -> Option<TerminalOutputResponse> {
        // 1. Get session
        let sessions = self.sessions.lock().await;
        let session = sessions.get(&pid)?;

        // 2. Get screen, calculate pagination, and collect ONLY requested range
        let (lines, total_lines, start, end, is_complete) = {
            let terminal = session.terminal.read().await;
            let screen = terminal.screen()?;
            let (_rows, cols) = screen.size();

            // Count total lines in scrollback buffer (NOT viewport size)
            // screen.size() returns viewport (24x80), not actual buffer size
            // Must iterate to count, but this is cheap (no string allocation)
            let total = screen.rows(0, cols).count();

            // Calculate pagination range
            let (start, end) = if offset < 0 {
                // Negative offset: tail behavior (last N lines)
                let tail_count = usize::try_from(-offset).unwrap_or(0).min(total);
                let start_pos = total.saturating_sub(tail_count);
                (start_pos, total)
            } else {
                // Positive offset: range read (offset..offset+length)
                let start_pos = usize::try_from(offset).unwrap_or(0).min(total);
                let end_pos = (start_pos + length).min(total);
                (start_pos, end_pos)
            };

            // Collect ONLY the requested range (massive memory savings)
            let lines: Vec<String> = screen.rows(0, cols).skip(start).take(end - start).collect();

            let complete = terminal.is_pty_closed();
            (lines, total, start, end, complete)
        }; // Read lock automatically dropped here

        // 4. Get exit code with write lock (separate, minimal critical section)
        let exit_code = if is_complete {
            let mut terminal = session.terminal.write().await;
            terminal
                .try_wait()
                .await
                .ok()
                .flatten()
                .map(|status| i32::from(!status.success()))
        } else {
            None
        };

        let has_more = end < total_lines || !is_complete;

        // 7. Update last read time
        *session.last_read_time.write().await = Instant::now();

        Some(TerminalOutputResponse {
            pid,
            lines,
            total_lines,
            lines_returned: end - start,
            is_complete,
            exit_code,
            has_more,
            buffer_truncated: Some(false), // VT100 scrollback handles truncation
        })
    }

    /// Send input to a running command
    ///
    /// Sends text to the PTY with optional newline appending.
    ///
    /// # Parameters
    /// - `pid`: Process ID
    /// - `input`: Text to send
    /// - `append_newline`: If true, appends '\n' to execute command (default: true)
    ///
    /// # Returns
    /// Ok(true) if successful, Err if session not found
    pub async fn send_input(
        &self,
        pid: u32,
        input: &str,
        append_newline: bool,
    ) -> Result<bool, anyhow::Error> {
        // 1. Get session (clone to release lock quickly)
        let sessions = self.sessions.lock().await;
        let session = sessions
            .get(&pid)
            .ok_or_else(|| anyhow::anyhow!("Process {pid} not found"))?
            .clone();
        drop(sessions); // Release lock before async PTY call

        // 2. Send to PTY terminal with conditional newline
        let terminal = session.terminal.read().await;

        let bytes = if append_newline {
            // Avoid intermediate String allocation from format!
            let mut buf = Vec::with_capacity(input.len() + 1);
            buf.extend_from_slice(input.as_bytes());
            buf.push(b'\n');
            Bytes::from(buf)
        } else {
            // Direct copy without intermediate Vec allocation
            Bytes::copy_from_slice(input.as_bytes())
        };

        terminal.send_input(bytes).await?;
        drop(terminal);

        log::debug!("Input sent: pid={}, bytes={}", pid, input.len());

        // 3. Update session state
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&pid) {
            session.is_blocked = false;
            session.ready_for_input = false;
        }

        Ok(true)
    }
}

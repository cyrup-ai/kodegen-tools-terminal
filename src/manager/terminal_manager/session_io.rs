//! Session I/O operations
//!
//! This module handles input/output operations for terminal sessions:
//! - Reading paginated output from Alacritty Grid with pagination
//! - Sending interactive input to PTY

use super::types::TerminalOutputResponse;
use bytes::Bytes;
use std::time::Instant;
use alacritty_terminal::index::{Line, Column};
use alacritty_terminal::grid::Dimensions;

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

        // 2. Get Grid, calculate pagination, and extract lines using direct indexing
        let (lines, total_lines, start, end, is_complete) = {
            let terminal = session.terminal.read().await;
            let term = terminal.term.read();  // parking_lot::RwLock
            let grid = term.grid();

            // Get grid dimensions
            let _screen_lines = grid.screen_lines();
            let history_size = grid.history_size();
            let columns = grid.columns();

            // Determine total content using cursor position (like current vt100 approach)
            let cursor_point = grid.cursor.point;
            let total = if cursor_point.line.0 == 0 && cursor_point.column.0 == 0 {
                0  // No content written yet
            } else if cursor_point.line.0 >= 0 {
                // Cursor in visible region: Line(0) = first visible, Line(n) = nth visible
                history_size + (cursor_point.line.0 as usize) + 1
            } else {
                // Cursor in scrollback (rare): Line(-1) = last scrollback, Line(-n) = nth from bottom
                history_size - ((-cursor_point.line.0) as usize) + 1
            };

            // Calculate pagination range (SAME LOGIC as current vt100 implementation)
            let (start, end) = if offset < 0 {
                // Negative offset: tail behavior (last N lines)
                let tail_count = ((-offset) as usize).min(total);
                (total.saturating_sub(tail_count), total)
            } else {
                // Positive offset: range read (offset..offset+length)
                let start_pos = (offset as usize).min(total);
                let end_pos = (start_pos + length).min(total);
                (start_pos, end_pos)
            };

            // Extract lines efficiently using direct Grid indexing
            let mut lines = Vec::with_capacity(end.saturating_sub(start));

            for abs_idx in start..end {
                // Convert absolute index to Line(i32) for Grid access
                // Absolute indexing: [0..history_size) = scrollback, [history_size..total) = visible
                // Line(i32) indexing: Line(-history..−1) = scrollback, Line(0..screen-1) = visible
                let line_idx = if abs_idx < history_size {
                    // In scrollback region
                    Line(-((history_size - abs_idx) as i32))
                } else {
                    // In visible region
                    Line((abs_idx - history_size) as i32)
                };

                // Get row from grid (zero-copy reference)
                let row = &grid[line_idx];

                // Render row to string
                let mut line_str = String::with_capacity(columns);
                for col in 0..columns {
                    line_str.push(row[Column(col)].c);
                }

                lines.push(line_str);
            }

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
            session.still_running = false;
            session.ready_for_input = false;
        }

        Ok(true)
    }
}

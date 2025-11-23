//! Session I/O operations
//!
//! This module handles input/output operations for terminal sessions:
//! - Reading paginated output from Alacritty Grid with pagination
//! - Sending interactive input to PTY

use super::types::TerminalOutputResponse;
use std::time::Instant;
use alacritty_terminal::index::{Line, Column};
use alacritty_terminal::grid::Dimensions;

impl super::TerminalManager {
    /// Get paginated output from a running command
    ///
    /// Extracts text from the VT100 screen buffer with pagination support.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    /// - `offset`: Starting line (negative = tail from end)
    /// - `length`: Maximum lines to return
    ///
    /// # Returns
    /// Terminal output with pagination info, or None if session not found
    pub async fn get_output(
        &self,
        connection_id: &str,
        terminal_id: u32,
        offset: i64,
        length: usize,
    ) -> Option<TerminalOutputResponse> {
        // 1. Get session using composite key
        let sessions = self.sessions.lock().await;
        let key = (connection_id.to_string(), terminal_id);
        let session = sessions.get(&key)?;

        // 2. Get Grid, calculate pagination, and extract lines using direct indexing
        let (lines, total_lines, start, end) = {
            let terminal = session.terminal.read().await;
            let term_arc = terminal.term.clone();

            // Use spawn_blocking to access FairMutex from async context
            tokio::task::spawn_blocking(move || {
                let term = term_arc.lock_unfair();
                let grid = term.grid();

                // Get grid dimensions
                let history_size = grid.history_size();
                let columns = grid.columns();

            log::debug!("get_output: grid dimensions - history_size={}, columns={}, screen_lines={}", 
                       history_size, columns, grid.screen_lines());

            // Determine total content by scanning grid for last non-empty line
            // NOTE: Cursor position is NOT reliable - it only shows where next char would be written,
            // not how much content exists. We must scan the grid itself.
            let screen_lines = grid.screen_lines();
            
            // Find the last line with content by scanning from bottom to top
            let mut last_content_line: Option<usize> = None;
            log::debug!("get_output: scanning grid for content...");
            
            // First check visible region (bottom to top)
            for line_idx in (0..screen_lines).rev() {
                let line = &grid[Line(line_idx as i32)];
                // Check if line has any non-whitespace, non-null characters
                let has_content = (0..columns).any(|col| {
                    let ch = line[Column(col)].c;
                    ch != '\0' && !ch.is_whitespace()
                });
                
                if has_content {
                    last_content_line = Some(history_size + line_idx);
                    log::debug!("get_output: found content in visible region at line_idx={}, abs_idx={}", 
                               line_idx, history_size + line_idx);
                    break;
                }
            }
            
            log::debug!("get_output: visible region scan complete, last_content_line={:?}", last_content_line);
            
            // If no content in visible region, check scrollback (bottom to top)
            if last_content_line.is_none() && history_size > 0 {
                log::debug!("get_output: scanning scrollback region...");
                for i in (0..history_size).rev() {
                    let line_idx = Line(-((history_size - i) as i32));
                    let line = &grid[line_idx];
                    let has_content = (0..columns).any(|col| {
                        let ch = line[Column(col)].c;
                        ch != '\0' && !ch.is_whitespace()
                    });
                    
                    if has_content {
                        last_content_line = Some(i);
                        break;
                    }
                }
            }
            
            // Total lines = last content line + 1 (or 0 if no content)
            let total = last_content_line.map(|idx| idx + 1).unwrap_or(0);

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

            (lines, total, start, end)
        }).await.ok()?
        }; // Read lock automatically dropped here

        // 3. Detect prompt and update still_running
        let output_text = lines.join("\n");
        let ready_for_input = super::repl_detection::detect_repl_ready(&output_text);
        let is_complete = ready_for_input;

        // 4. Get CWD from TerminalManager::get_terminal_cwd
        let cwd = self.get_terminal_cwd(connection_id, terminal_id).await;

        // 5. Update session state and get exit code (with sessions lock held)
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(&(connection_id.to_string(), terminal_id))?;

        // Update prompt detection results
        session.still_running = !ready_for_input;
        session.ready_for_input = ready_for_input;

        // Update last read time
        *session.last_read_time.write().await = Instant::now();

        // Get exit code if command completed
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

        Some(TerminalOutputResponse {
            connection_id: connection_id.to_string(),
            terminal_id,
            lines,
            total_lines,
            lines_returned: end - start,
            is_complete,
            exit_code,
            has_more,
            buffer_truncated: Some(false), // VT100 scrollback handles truncation
            cwd,
        })
    }

    /// Send input to a running command
    ///
    /// Sends text to the PTY with optional newline appending.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    /// - `input`: Text to send
    /// - `append_newline`: If true, appends '\n' to execute command (default: true)
    ///
    /// # Returns
    /// Ok(true) if successful, Err if session not found
    pub async fn send_input(
        &self,
        connection_id: &str,
        terminal_id: u32,
        input: &str,
        append_newline: bool,
    ) -> Result<bool, anyhow::Error> {
        // 1. Get session (clone to release lock quickly)
        let sessions = self.sessions.lock().await;
        let key = (connection_id.to_string(), terminal_id);
        let session = sessions
            .get(&key)
            .ok_or_else(|| anyhow::anyhow!(
                "Terminal not found: connection_id={}, terminal_id={}",
                connection_id,
                terminal_id
            ))?
            .clone();
        drop(sessions); // Release lock before async PTY call

        // 2. Send to PTY terminal with conditional newline
        let terminal = session.terminal.read().await;

        let bytes = if append_newline {
            let mut buf = Vec::with_capacity(input.len() + 1);
            buf.extend_from_slice(input.as_bytes());
            buf.push(b'\n');
            buf
        } else {
            input.as_bytes().to_vec()
        };

        terminal.send_input(bytes).await?;
        drop(terminal);

        log::debug!("Input sent: connection_id={}, terminal_id={}, bytes={}", 
                   connection_id, terminal_id, input.len());

        // 3. Update session state
        let mut sessions = self.sessions.lock().await;
        if let Some(session) = sessions.get_mut(&key) {
            session.still_running = false;
            session.ready_for_input = false;
        }

        Ok(true)
    }

    /// Subscribe to real-time output broadcast for a terminal session
    ///
    /// Returns a broadcast receiver that receives Alacritty Grid snapshots after each VTE update.
    /// This enables real-time streaming of terminal output to clients.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    ///
    /// # Returns
    /// Broadcast receiver for output, or None if session not found
    ///
    /// # Notes
    /// - The broadcast channel may lag under heavy output (RecvError::Lagged)
    /// - Lagged messages are acceptable for streaming (best-effort delivery)
    /// - For complete, authoritative output, always use `get_output()` after completion
    /// - This provides real-time UX; Grid via get_output() is the single source of truth
    pub async fn subscribe_output(
        &self,
        connection_id: &str,
        terminal_id: u32,
    ) -> Option<tokio::sync::broadcast::Receiver<()>> {
        let sessions = self.sessions.lock().await;
        let key = (connection_id.to_string(), terminal_id);
        let session = sessions.get(&key)?;

        // Get Terminal and call subscribe_output()
        let terminal = session.terminal.read().await;
        Some(terminal.subscribe_output())
    }
}

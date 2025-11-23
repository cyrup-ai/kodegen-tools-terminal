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

        // 3. Get CWD from TerminalManager::get_terminal_cwd
        let cwd = self.get_terminal_cwd(connection_id, terminal_id).await;

        // 4. Update session state and get exit code (with sessions lock held)
        let mut sessions = self.sessions.lock().await;
        let session = sessions.get_mut(&(connection_id.to_string(), terminal_id))?;

        // Update last read time
        *session.last_read_time.write().await = Instant::now();

        // Try to get exit code (best effort, non-blocking)
        let exit_code = {
            let mut terminal = session.terminal.write().await;
            terminal.try_wait().await.ok().flatten()
        };

        let is_complete = false;
        let has_more = end < total_lines;

        Some(TerminalOutputResponse {
            connection_id: connection_id.to_string(),
            terminal_id,
            lines,
            total_lines,
            lines_returned: end - start,
            is_complete,
            exit_code,
            has_more,
            buffer_truncated: Some(false),
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

    /// Subscribe to bell (BEL/\x07) events for a terminal session
    ///
    /// Returns a broadcast receiver that receives notifications when terminal receives BEL character.
    /// Used for command completion detection when commands are wrapped with `;printf '\x07'`.
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier from stdio server
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    ///
    /// # Returns
    /// Broadcast receiver for bell events, or None if session not found
    pub async fn subscribe_bell(
        &self,
        connection_id: &str,
        terminal_id: u32,
    ) -> Option<tokio::sync::broadcast::Receiver<()>> {
        let sessions = self.sessions.lock().await;
        let key = (connection_id.to_string(), terminal_id);
        let session = sessions.get(&key)?;

        // Get Terminal and call subscribe_bell()
        let terminal = session.terminal.read().await;
        Some(terminal.subscribe_bell())
    }

    /// Execute command with bell-based completion detection and real-time streaming
    ///
    /// This is the main method for executing commands in persistent terminal sessions.
    /// It handles:
    /// - Terminal creation/reuse
    /// - Command wrapping with bell marker for completion detection
    /// - Real-time output streaming
    /// - Timeout and cancellation
    ///
    /// # Parameters
    /// - `connection_id`: Connection identifier
    /// - `terminal_id`: Terminal number (1, 2, 3...)
    /// - `command`: Shell command to execute
    /// - `timeout`: Maximum execution time
    /// - `is_cancelled`: Cancellation check function
    /// - `stream_callback`: Async callback for streaming output updates (can be no-op)
    ///
    /// # Returns
    /// Final output with exit code and metadata
    pub async fn execute_command_with_completion<F, Fut>(
        &self,
        connection_id: &str,
        terminal_id: u32,
        command: &str,
        timeout: std::time::Duration,
        is_cancelled: F,
        mut stream_callback: impl FnMut(String) -> Fut,
    ) -> Result<TerminalOutputResponse, anyhow::Error>
    where
        F: Fn() -> bool,
        Fut: std::future::Future<Output = ()>,
    {
        let start = Instant::now();

        // ========== PHASE 1: TERMINAL SETUP ==========

        // Check if terminal exists, create if needed
        let terminal_exists = self.get_session(connection_id, terminal_id)
            .await
            .is_some();

        if !terminal_exists {
            // Create new interactive shell (no command sent yet)
            self.spawn_command(connection_id, terminal_id, None).await?;
        }

        // ========== PHASE 2: STREAMING SETUP ==========

        // Subscribe to broadcast channels BEFORE sending command (avoid race condition)
        let mut output_rx = self.subscribe_output(connection_id, terminal_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Terminal session not found after creation"))?;

        let mut bell_rx = self.subscribe_bell(connection_id, terminal_id)
            .await
            .ok_or_else(|| anyhow::anyhow!("Terminal session not found for bell subscription"))?;

        // Wrap command with bell marker for completion detection
        // Format: (command); printf '\x07'
        // - Subshell isolates command execution
        // - ; (not &&) ensures bell fires even if command fails
        // - \x07 is BEL character that triggers bell event
        let wrapped_command = format!("({}); printf '\\x07'", command);

        // Send wrapped command to shell
        self.send_input(connection_id, terminal_id, &wrapped_command, true).await?;

        // ========== PHASE 3: REAL-TIME STREAMING LOOP ==========

        let mut last_output = String::new();

        loop {
            // Check cancellation
            if is_cancelled() {
                self.force_terminate(connection_id, terminal_id).await.ok();
                return Err(anyhow::anyhow!("Command execution cancelled by user"));
            }

            // Check timeout
            if start.elapsed() > timeout {
                // Get final output before terminating
                let final_output = self.get_output(connection_id, terminal_id, 0, usize::MAX)
                    .await
                    .map(|r| r.lines.join("\n"))
                    .unwrap_or_else(|| last_output.clone());

                self.force_terminate(connection_id, terminal_id).await.ok();

                return Err(anyhow::anyhow!(
                    "Command timed out after {:?}. Last output:\n{}",
                    timeout,
                    final_output
                ));
            }

            // Try to receive output or bell notification (non-blocking with timeout)
            tokio::select! {
                // Output update notification
                result = output_rx.recv() => {
                    use tokio::sync::broadcast::error::RecvError;
                    match result {
                        Ok(()) => {
                            // Screen updated - get actual content
                            let screen_content = self.get_output(connection_id, terminal_id, 0, usize::MAX)
                                .await
                                .map(|r| r.lines.join("\n"))
                                .unwrap_or_default();

                            last_output = screen_content.clone();

                            // Truncate for streaming (last 30 lines or 2000 chars)
                            let display = truncate_for_streaming(&screen_content);

                            // Stream to callback (fire-and-forget)
                            stream_callback(display).await;
                        }
                        Err(RecvError::Lagged(n)) => {
                            // Missed some messages due to lag - resubscribe and continue
                            log::warn!("Output stream lagged by {} messages (resubscribing)", n);

                            output_rx = self.subscribe_output(connection_id, terminal_id)
                                .await
                                .ok_or_else(|| anyhow::anyhow!("Terminal closed"))?;
                        }
                        Err(RecvError::Closed) => {
                            // Channel closed - terminal finished
                            log::info!("Output broadcast channel closed");
                            break;
                        }
                    }
                }

                // Bell event (command completion marker)
                result = bell_rx.recv() => {
                    use tokio::sync::broadcast::error::RecvError;
                    match result {
                        Ok(()) => {
                            log::info!("Bell event received - command completed");
                            // Get final output before breaking
                            let screen_content = self.get_output(connection_id, terminal_id, 0, usize::MAX)
                                .await
                                .map(|r| r.lines.join("\n"))
                                .unwrap_or_default();
                            last_output = screen_content;
                            break;
                        }
                        Err(RecvError::Lagged(_)) => {
                            // Bell channel lagged - resubscribe
                            bell_rx = self.subscribe_bell(connection_id, terminal_id)
                                .await
                                .ok_or_else(|| anyhow::anyhow!("Terminal closed"))?;
                        }
                        Err(RecvError::Closed) => {
                            log::info!("Bell broadcast channel closed");
                            break;
                        }
                    }
                }

                // Timeout after 100ms of no events
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    // No events - continue polling
                }
            }
        }

        // ========== PHASE 4: FINAL OUTPUT COLLECTION ==========

        // Get complete, authoritative output from Alacritty Grid
        let output_response = self.get_output(connection_id, terminal_id, 0, usize::MAX)
            .await
            .ok_or_else(|| anyhow::anyhow!("Terminal not found after completion"))?;

        Ok(output_response)
    }
}

/// Truncate output for streaming to avoid overwhelming the client
///
/// Shows last 30 lines or 2000 chars, whichever is smaller.
/// Final output via get_output() is never truncated.
fn truncate_for_streaming(content: &str) -> String {
    const MAX_STREAM_CHARS: usize = 2000;
    const MAX_STREAM_LINES: usize = 30;

    if content.len() <= MAX_STREAM_CHARS {
        return content.to_string();
    }

    let lines: Vec<&str> = content.lines().collect();

    if lines.len() <= MAX_STREAM_LINES {
        return content.to_string();
    }

    // Show last N lines
    let tail_lines = &lines[lines.len().saturating_sub(MAX_STREAM_LINES)..];
    format!(
        "...\n[{} earlier lines omitted for streaming]\n{}",
        lines.len() - tail_lines.len(),
        tail_lines.join("\n")
    )
}

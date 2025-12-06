//! VteProcessor thread - processes VTE sequences and maintains terminal grid

use crate::pty::terminal::events::{ShellOutput, TerminalBuffer};
use crate::pty::terminal::sync::FairMutex;
use crate::pty::terminal::EventBridge;
use alacritty_terminal::term::{Term, Config as AlacrittyConfig};
use alacritty_terminal::term::cell::{Flags, LineLength};
use alacritty_terminal::index::{Line, Column};
use alacritty_terminal::grid::{Dimensions, Grid, GridCell};
use alacritty_terminal::term::cell::Cell;
use vte::ansi::{ClearMode, Handler, Mode as AnsiMode, NamedMode};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::broadcast;

/// Extract lines from grid, trimming trailing whitespace from each line
fn extract_lines_from_grid(grid: &Grid<Cell>) -> Vec<String> {
    let history_size = grid.history_size();
    let screen_lines = grid.screen_lines();

    let mut lines = Vec::new();
    let mut current_line = String::new();

    for line_idx in (-(history_size as i32))..(screen_lines as i32) {
        let line = &grid[Line(line_idx)];
        let content_length = line.line_length().0;

        for col_idx in 0..content_length {
            let cell = &line[Column(col_idx)];
            if !cell.flags().contains(Flags::WIDE_CHAR_SPACER) {
                current_line.push(cell.c);
            }
        }

        let last_col = grid.columns().saturating_sub(1);
        let wraps = line[Column(last_col)].flags().contains(Flags::WRAPLINE);

        if !wraps {
            lines.push(current_line.trim_end().to_string());
            current_line.clear();
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line.trim_end().to_string());
    }

    // Trim trailing empty lines
    while let Some(last) = lines.last() {
        if last.is_empty() {
            lines.pop();
        } else {
            break;
        }
    }

    lines
}

/// Handle to control the VteProcessor thread
pub struct VteHandle {
    pub buffer_tx: broadcast::Sender<TerminalBuffer>,
    pub term: Arc<FairMutex<Term<EventBridge>>>,
    pub current_cwd: Arc<RwLock<PathBuf>>,
    pub last_exit_code: Arc<RwLock<Option<i32>>>,
}

impl VteHandle {
    /// Read current grid state directly (for READ/LIST actions)
    ///
    /// Returns (lines, cwd, exit_code) tuple with the current terminal buffer content,
    /// working directory, and last known exit code.
    pub fn read_grid(&self, tail: u32) -> (Vec<String>, String, Option<i32>) {
        let term = self.term.lock_unfair();
        let lines = extract_lines_from_grid(term.grid());

        // Apply tail limit
        let output_lines = if tail > 0 && lines.len() > tail as usize {
            lines[lines.len() - tail as usize..].to_vec()
        } else {
            lines
        };

        // Get cwd from shared state
        let cwd = self.current_cwd.read()
            .map(|guard| guard.display().to_string())
            .unwrap_or_else(|_| "/".to_string());

        // Get last exit code from shared state
        let exit_code = self.last_exit_code.read()
            .map(|guard| *guard)
            .unwrap_or(None);

        (output_lines, cwd, exit_code)
    }

    /// Clear the entire grid (history + viewport + cursor)
    ///
    /// This ensures read_grid() returns a clean slate after clearing.
    /// Used when the `clear` parameter is true before executing a command.
    pub fn clear_grid(&self) {
        let mut term = self.term.lock_unfair();
        // Use clear_screen instead of grid reset - preserves terminal modes like LINE_FEED_NEW_LINE
        // Order matters: All scrolls viewport to history, then Saved clears that history
        term.clear_screen(ClearMode::All);    // Clear viewport (scrolls to history first)
        term.clear_screen(ClearMode::Saved);  // Clear scrollback (including what was just scrolled)
    }
}

/// VteProcessor thread implementation
///
/// Owns its own Term exclusively. Subscribes to ShellOutput events,
/// processes VTE sequences, extracts terminal buffer, emits TerminalBuffer events.
pub struct VteProcessorThread {
    parser: vte::ansi::Processor,
    term: Arc<FairMutex<Term<EventBridge>>>,
    shell_output_rx: broadcast::Receiver<ShellOutput>,
    buffer_tx: broadcast::Sender<TerminalBuffer>,
    current_cwd: Arc<RwLock<PathBuf>>,
    last_exit_code: Arc<RwLock<Option<i32>>>,
}

impl VteProcessorThread {
    pub fn spawn(
        shell_output_rx: broadcast::Receiver<ShellOutput>,
        initial_cwd: PathBuf,
        term_size: super::TermSize,
    ) -> (VteHandle, tokio::task::JoinHandle<()>) {
        let (buffer_tx, _) = broadcast::channel(1024);

        // VteProcessor creates and owns its Term exclusively
        let event_bridge = EventBridge::new(buffer_tx.clone());
        let alacritty_config = AlacrittyConfig {
            scrolling_history: term_size.scrollback,
            ..Default::default()
        };
        let mut term = Term::new(alacritty_config, &term_size, event_bridge);

        // Enable LINE_FEED_NEW_LINE mode so LF also does CR (like ONLCR in a real PTY)
        // This ensures cursor resets to column 0 on newlines, matching expected terminal behavior
        term.set_mode(AnsiMode::Named(NamedMode::LineFeedNewLine));

        let term = Arc::new(FairMutex::new(term));

        let parser = vte::ansi::Processor::new();
        let current_cwd = Arc::new(RwLock::new(initial_cwd));
        let last_exit_code = Arc::new(RwLock::new(None));

        let thread_impl = Self {
            parser,
            term: term.clone(),
            shell_output_rx,
            buffer_tx: buffer_tx.clone(),
            current_cwd: current_cwd.clone(),
            last_exit_code: last_exit_code.clone(),
        };

        let join_handle = tokio::spawn(async move {
            thread_impl.run().await;
        });

        let vte_handle = VteHandle {
            buffer_tx,
            term,
            current_cwd,
            last_exit_code,
        };

        (vte_handle, join_handle)
    }

    async fn run(mut self) {
        log::debug!("VteProcessor task starting");

        loop {
            log::debug!("VteProcessor: waiting for next event");
            match self.shell_output_rx.recv().await {
                Ok(ShellOutput::Shutdown) => {
                    log::debug!("VteProcessor: received Shutdown event, exiting");
                    break;
                }
                Ok(event) => {
                    log::debug!("VteProcessor: recv() returned an event");
                    self.process_shell_output(event);
                    log::debug!("VteProcessor: finished processing event");
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    log::warn!("VteProcessor lagged, skipped {} events", skipped);
                }
                Err(broadcast::error::RecvError::Closed) => {
                    log::debug!("VteProcessor: channel closed, exiting");
                    break;
                }
            }
        }
        log::debug!("VteProcessor task stopping");
    }

    fn process_shell_output(&mut self, event: ShellOutput) {
        log::debug!("VteProcessor: received ShellOutput event");
        match event {
            ShellOutput::Bytes { request_id, data } => {
                log::debug!("VteProcessor: processing {} bytes for request_id={:?}", data.len(), request_id);

                // Convert LF to CRLF so cursor returns to column 0 on newlines
                // This is necessary because LF only moves cursor down (linefeed),
                // it doesn't return to column 0. Without CR, content gets written
                // at the cursor's current column position, causing indentation issues.
                let mut transformed = Vec::with_capacity(data.len() + data.iter().filter(|&&b| b == b'\n').count());
                for &byte in &data {
                    if byte == b'\n' {
                        transformed.push(b'\r');
                    }
                    transformed.push(byte);
                }

                // Reserve fairness lock (prevents API starvation)
                let _lease = self.term.lease();

                // Try to acquire data lock (non-blocking)
                let mut term = match self.term.try_lock_unfair() {
                    Some(t) => t,
                    None => return, // Locked by API, skip this batch
                };

                // Process VTE sequences with transformed data
                self.parser.advance(&mut *term, &transformed);
                drop(term);

                // Emit incremental update
                self.emit_buffer_update(request_id, 0, false);
            }
            ShellOutput::ExecComplete { request_id, exit_code, cwd } => {
                log::debug!("VteProcessor: ExecComplete for request_id={:?}, exit_code={}", request_id, exit_code);
                // Update CWD tracking (write to shared RwLock)
                if let Ok(mut cwd_guard) = self.current_cwd.write() {
                    *cwd_guard = cwd;
                }
                // Update exit code tracking (write to shared RwLock)
                if let Ok(mut exit_guard) = self.last_exit_code.write() {
                    *exit_guard = Some(exit_code as i32);
                }

                // Emit final update with is_final=true
                self.emit_buffer_update(request_id, exit_code as i32, true);
            }
            ShellOutput::Shutdown => {
                // This is already handled in run() loop, should never reach here
                log::warn!("VteProcessor: received Shutdown in process_shell_output (should be handled in run loop)");
            }
        }
    }

    fn emit_buffer_update(&self, request_id: rmcp::model::RequestId, exit_code: i32, is_final: bool) {
        log::debug!("VteProcessor: emit_buffer_update request_id={:?}, exit_code={}, is_final={}", request_id, exit_code, is_final);
        // Acquire unfair lock for reading grid (like Alacritty does)
        let term = self.term.lock_unfair();
        let lines = extract_lines_from_grid(term.grid());

        // Get cursor position
        let cursor = term.grid().cursor.point;
        drop(term);

        // Read current cwd from shared state
        let cwd = self.current_cwd.read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| PathBuf::from("/"));

        // Emit TerminalBuffer::Updated event
        let _ = self.buffer_tx.send(TerminalBuffer::Updated {
            request_id,
            lines,
            cursor_line: cursor.line.0 as usize,
            cursor_col: cursor.column.0,
            cwd,
            exit_code,
            is_final,
        });
    }
}

//! VteProcessor thread - processes VTE sequences and maintains terminal grid

use crate::pty::terminal::events::{ShellOutput, TerminalBuffer};
use crate::pty::terminal::EventBridge;
use alacritty_terminal::term::{Term, Config as AlacrittyConfig};
use alacritty_terminal::term::cell::{Flags, LineLength};
use alacritty_terminal::index::{Line, Column};
use alacritty_terminal::grid::{Dimensions, Grid, GridCell};
use alacritty_terminal::term::cell::Cell;
use vte::ansi::{ClearMode, Handler};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc, oneshot};

/// Request to read current grid state
pub struct ReadGridRequest {
    pub tail: u32,
    pub response_tx: oneshot::Sender<GridSnapshot>,
}

/// Request to clear the grid
pub struct ClearGridRequest {
    pub response_tx: oneshot::Sender<()>,
}

/// Response containing grid state snapshot
#[derive(Debug, Clone)]
pub struct GridSnapshot {
    pub lines: Vec<String>,
    pub cwd: String,
    pub exit_code: Option<i32>,
}

/// Normalize newlines: replace lone LF (\n) with CRLF (\r\n)
///
/// Since we don't use a real PTY (no ONLCR termios flag), the shell sends
/// raw LF without CR. This function implements ONLCR-like behavior so the
/// terminal cursor properly resets to column 0 on each newline.
fn normalize_newlines(data: &[u8]) -> Vec<u8> {
    let mut result = Vec::with_capacity(data.len() + data.len() / 10);
    let mut prev_was_cr = false;

    for &byte in data {
        if byte == b'\n' && !prev_was_cr {
            // Lone LF - add CR before it
            result.push(b'\r');
        }
        result.push(byte);
        prev_was_cr = byte == b'\r';
    }

    result
}

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
///
/// NO FairMutex - all grid access goes through async channels
pub struct VteHandle {
    pub buffer_tx: broadcast::Sender<TerminalBuffer>,
    /// Channel to request grid reads from VteProcessor
    read_request_tx: mpsc::Sender<ReadGridRequest>,
    /// Channel to request grid clears from VteProcessor
    clear_request_tx: mpsc::Sender<ClearGridRequest>,
    pub current_cwd: Arc<RwLock<PathBuf>>,
    pub last_exit_code: Arc<RwLock<Option<i32>>>,
}

impl VteHandle {
    /// Read current grid state via async channel (NO BLOCKING)
    ///
    /// Sends request to VteProcessor, awaits response via oneshot channel.
    /// VteProcessor handles request inline in its event loop.
    pub async fn read_grid(&self, tail: u32) -> Result<GridSnapshot, anyhow::Error> {
        let (response_tx, response_rx) = oneshot::channel();

        self.read_request_tx
            .send(ReadGridRequest { tail, response_tx })
            .await
            .map_err(|_| anyhow::anyhow!("VteProcessor terminated - cannot read grid"))?;

        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("VteProcessor dropped response channel"))
    }

    /// Clear the entire grid via async channel (NO BLOCKING)
    pub async fn clear_grid(&self) -> Result<(), anyhow::Error> {
        let (response_tx, response_rx) = oneshot::channel();

        self.clear_request_tx
            .send(ClearGridRequest { response_tx })
            .await
            .map_err(|_| anyhow::anyhow!("VteProcessor terminated - cannot clear grid"))?;

        response_rx
            .await
            .map_err(|_| anyhow::anyhow!("VteProcessor dropped response channel"))
    }
}

/// VteProcessor thread implementation
///
/// OWNS Term exclusively - no shared FairMutex.
/// Receives grid read/clear requests via mpsc channels.
pub struct VteProcessorThread {
    parser: vte::ansi::Processor,
    term: Term<EventBridge>,  // OWNED directly, not Arc<FairMutex<...>>
    shell_output_rx: broadcast::Receiver<ShellOutput>,
    buffer_tx: broadcast::Sender<TerminalBuffer>,
    /// Channel to receive grid read requests
    read_request_rx: mpsc::Receiver<ReadGridRequest>,
    /// Channel to receive grid clear requests
    clear_request_rx: mpsc::Receiver<ClearGridRequest>,
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

        // Create request channels
        let (read_request_tx, read_request_rx) = mpsc::channel(32);
        let (clear_request_tx, clear_request_rx) = mpsc::channel(8);

        // VteProcessor OWNS its Term directly (no Arc, no FairMutex)
        let event_bridge = EventBridge::new(buffer_tx.clone());
        let alacritty_config = AlacrittyConfig {
            scrolling_history: term_size.scrollback,
            ..Default::default()
        };
        let term = Term::new(alacritty_config, &term_size, event_bridge);
        // Note: We use normalize_newlines() to convert LF to CRLF before parsing
        // LineFeedNewLine mode is unreliable because escape sequences can reset it

        let parser = vte::ansi::Processor::new();
        let current_cwd = Arc::new(RwLock::new(initial_cwd));
        let last_exit_code = Arc::new(RwLock::new(None));

        let thread_impl = Self {
            parser,
            term,  // Owned directly
            shell_output_rx,
            buffer_tx: buffer_tx.clone(),
            read_request_rx,
            clear_request_rx,
            current_cwd: current_cwd.clone(),
            last_exit_code: last_exit_code.clone(),
        };

        let join_handle = tokio::spawn(async move {
            thread_impl.run().await;
        });

        let vte_handle = VteHandle {
            buffer_tx,
            read_request_tx,
            clear_request_tx,
            current_cwd,
            last_exit_code,
        };

        (vte_handle, join_handle)
    }

    async fn run(mut self) {
        log::debug!("VteProcessor task starting");

        loop {
            tokio::select! {
                biased;

                // Priority 1: Handle read requests (fast path for API)
                Some(request) = self.read_request_rx.recv() => {
                    self.handle_read_request(request);
                }

                // Priority 2: Handle clear requests
                Some(request) = self.clear_request_rx.recv() => {
                    self.handle_clear_request(request);
                }

                // Priority 3: Process shell output
                result = self.shell_output_rx.recv() => {
                    match result {
                        Ok(ShellOutput::Shutdown) => {
                            log::debug!("VteProcessor: received Shutdown event, exiting");
                            break;
                        }
                        Ok(event) => {
                            self.process_shell_output(event);
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
            }
        }
        log::debug!("VteProcessor task stopping");
    }

    /// Handle a grid read request - NO LOCK, we own the Term
    fn handle_read_request(&self, request: ReadGridRequest) {
        let lines = extract_lines_from_grid(self.term.grid());

        // Apply tail limit
        let lines = if request.tail > 0 && lines.len() > request.tail as usize {
            lines[lines.len() - request.tail as usize..].to_vec()
        } else {
            lines
        };

        let cwd = self.current_cwd.read()
            .map(|guard| guard.display().to_string())
            .unwrap_or_else(|_| "/".to_string());

        let exit_code = self.last_exit_code.read()
            .map(|guard| *guard)
            .unwrap_or(None);

        let _ = request.response_tx.send(GridSnapshot { lines, cwd, exit_code });
    }

    /// Handle a grid clear request - NO LOCK, we own the Term
    fn handle_clear_request(&mut self, request: ClearGridRequest) {
        self.term.clear_screen(ClearMode::All);
        self.term.clear_screen(ClearMode::Saved);
        let _ = request.response_tx.send(());
    }

    fn process_shell_output(&mut self, event: ShellOutput) {
        log::debug!("VteProcessor: received ShellOutput event");
        match event {
            ShellOutput::Bytes { request_id, data } => {
                log::debug!("VteProcessor: processing {} bytes for request_id={:?}", data.len(), request_id);

                // Convert LF to CRLF so cursor returns to column 0 on newlines
                let normalized = normalize_newlines(&data);

                // NO LOCK NEEDED - we own self.term exclusively
                self.parser.advance(&mut self.term, &normalized);

                // Emit incremental update
                self.emit_buffer_update(request_id, 0, false);
            }
            ShellOutput::ExecComplete { request_id, exit_code, cwd } => {
                log::debug!("VteProcessor: ExecComplete for request_id={:?}, exit_code={}", request_id, exit_code);
                if let Ok(mut cwd_guard) = self.current_cwd.write() {
                    *cwd_guard = cwd;
                }
                if let Ok(mut exit_guard) = self.last_exit_code.write() {
                    *exit_guard = Some(exit_code as i32);
                }
                self.emit_buffer_update(request_id, exit_code as i32, true);
            }
            ShellOutput::Shutdown => {
                log::warn!("VteProcessor: received Shutdown in process_shell_output (should be handled in run loop)");
            }
        }
    }

    fn emit_buffer_update(&self, request_id: rmcp::model::RequestId, exit_code: i32, is_final: bool) {
        log::debug!("VteProcessor: emit_buffer_update request_id={:?}, exit_code={}, is_final={}", request_id, exit_code, is_final);

        // NO LOCK NEEDED - we own self.term exclusively
        let lines = extract_lines_from_grid(self.term.grid());
        let cursor = self.term.grid().cursor.point;

        let cwd = self.current_cwd.read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| PathBuf::from("/"));

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

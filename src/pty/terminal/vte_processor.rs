//! VteProcessor thread - processes VTE sequences and maintains terminal grid

use crate::pty::terminal::events::{ShellOutput, TerminalBuffer};
use crate::pty::terminal::sync::FairMutex;
use crate::pty::terminal::EventBridge;
use alacritty_terminal::term::{Term, Config as AlacrittyConfig};
use alacritty_terminal::index::{Line, Column};
use alacritty_terminal::grid::Dimensions;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Handle to control the VteProcessor thread
pub struct VteHandle {
    pub buffer_tx: broadcast::Sender<TerminalBuffer>,
    pub shutdown_flag: Arc<AtomicBool>,
}

impl Drop for VteHandle {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
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
    shutdown_flag: Arc<AtomicBool>,
    current_cwd: PathBuf,
}

impl VteProcessorThread {
    pub fn spawn(
        shell_output_rx: broadcast::Receiver<ShellOutput>,
        initial_cwd: PathBuf,
        term_size: super::TermSize,
    ) -> (VteHandle, tokio::task::JoinHandle<()>) {
        let (buffer_tx, _) = broadcast::channel(1024);
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        // VteProcessor creates and owns its Term exclusively
        let event_bridge = EventBridge::new(buffer_tx.clone());
        let alacritty_config = AlacrittyConfig {
            scrolling_history: term_size.scrollback,
            ..Default::default()
        };
        let term = Term::new(alacritty_config, &term_size, event_bridge);
        let term = Arc::new(FairMutex::new(term));

        let parser = vte::ansi::Processor::new();

        let thread_impl = Self {
            parser,
            term,
            shell_output_rx,
            buffer_tx: buffer_tx.clone(),
            shutdown_flag: shutdown_flag.clone(),
            current_cwd: initial_cwd,
        };

        let join_handle = tokio::spawn(async move {
            thread_impl.run().await;
        });

        let vte_handle = VteHandle {
            buffer_tx,
            shutdown_flag,
        };

        (vte_handle, join_handle)
    }

    async fn run(mut self) {
        log::debug!("VteProcessor task starting");

        while !self.shutdown_flag.load(Ordering::Relaxed) {
            log::debug!("VteProcessor: waiting for next event");
            match self.shell_output_rx.recv().await {
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
                // Reserve fairness lock (prevents API starvation)
                let _lease = self.term.lease();

                // Try to acquire data lock (non-blocking)
                let mut term = match self.term.try_lock_unfair() {
                    Some(t) => t,
                    None => return, // Locked by API, skip this batch
                };

                // Process VTE sequences
                self.parser.advance(&mut *term, &data);
                drop(term);

                // Emit incremental update
                self.emit_buffer_update(request_id, 0, false);
            }
            ShellOutput::ExecComplete { request_id, exit_code, cwd } => {
                log::debug!("VteProcessor: ExecComplete for request_id={:?}, exit_code={}", request_id, exit_code);
                // Update CWD tracking
                self.current_cwd = cwd;

                // Emit final update with is_final=true
                self.emit_buffer_update(request_id, exit_code as i32, true);
            }
        }
    }

    fn emit_buffer_update(&self, request_id: rmcp::model::RequestId, exit_code: i32, is_final: bool) {
        log::debug!("VteProcessor: emit_buffer_update request_id={:?}, exit_code={}, is_final={}", request_id, exit_code, is_final);
        // Acquire unfair lock for reading grid (like Alacritty does)
        let term = self.term.lock_unfair();
        let grid = term.grid();

        // Extract lines from grid
        let mut lines = Vec::with_capacity(grid.screen_lines());
        for line_idx in 0..grid.screen_lines() {
            let line = &grid[Line(line_idx as i32)];
            let mut line_str = String::with_capacity(grid.columns());
            for col_idx in 0..grid.columns() {
                line_str.push(line[Column(col_idx)].c);
            }
            lines.push(line_str);
        }

        // Get cursor position
        let cursor = term.grid().cursor.point;
        drop(term);

        // Emit TerminalBuffer::Updated event
        let _ = self.buffer_tx.send(TerminalBuffer::Updated {
            request_id,
            lines,
            cursor_line: cursor.line.0 as usize,
            cursor_col: cursor.column.0,
            cwd: self.current_cwd.clone(),
            exit_code,
            is_final,
        });
    }
}

//! KodegenInteractive thread - manages persistent shell with streaming + cancellation
//!
//! Architecture:
//! - Uses kodegen_bash_shell's stream() API for command execution
//! - CancellationToken provides clean programmatic cancellation
//! - No PTY hackery - just clean async streams

use crate::pty::terminal::{ExecuteCommand, ShellOutput};
use crate::shell::KodegenShell;
use futures::StreamExt;
use kodegen_bash_shell::prelude::ShellVariable;
use kodegen_bash_shell::{CancellationToken, OutputStreamType};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};

/// Handle to control the KodegenInteractive thread
pub struct ShellHandle {
    pub command_tx: mpsc::Sender<ExecuteCommand>,
    pub output_tx: broadcast::Sender<ShellOutput>,
    /// Cancel channel - send () to cancel currently running command
    pub cancel_tx: mpsc::Sender<()>,
}

/// KodegenInteractive thread implementation
pub struct KodegenInteractiveThread {
    shell: KodegenShell,
    command_rx: mpsc::Receiver<ExecuteCommand>,
    cancel_rx: mpsc::Receiver<()>,
    output_tx: broadcast::Sender<ShellOutput>,
    /// Current cancellation token - set when command starts, cancelled on cancel_rx
    current_token: Arc<RwLock<Option<CancellationToken>>>,
    current_request_id: Arc<RwLock<rmcp::model::RequestId>>,
    /// Terminal columns (for COLUMNS environment variable)
    cols: u16,
    /// Terminal rows (for LINES environment variable)
    rows: u16,
}

impl KodegenInteractiveThread {
    pub async fn spawn(
        cols: u16,
        rows: u16,
        working_dir: Option<PathBuf>
    ) -> Result<(ShellHandle, tokio::task::JoinHandle<()>), std::io::Error> {
        log::debug!("KodegenInteractiveThread::spawn() called with working_dir={:?}", working_dir);

        // Create shell (no PTY needed - uses internal pipes)
        // Pass working_dir so shell starts in client's directory, not server's
        let mut shell = KodegenShell::new(working_dir).await?;
        log::debug!("KodegenShell created successfully");

        // Set COLUMNS and LINES environment variables for shell commands
        if let Err(e) = shell.shell_mut().env.set_global("COLUMNS", ShellVariable::new(cols.to_string())) {
            log::warn!("Failed to set COLUMNS during initialization: {}", e);
        }
        if let Err(e) = shell.shell_mut().env.set_global("LINES", ShellVariable::new(rows.to_string())) {
            log::warn!("Failed to set LINES during initialization: {}", e);
        }
        log::debug!("Set COLUMNS={} LINES={}", cols, rows);

        let (command_tx, command_rx) = mpsc::channel(32);
        let (cancel_tx, cancel_rx) = mpsc::channel(4);
        let (output_tx, _) = broadcast::channel(1024);

        // Create shared state
        let current_token = Arc::new(RwLock::new(None));
        let current_request_id = Arc::new(RwLock::new(rmcp::model::RequestId::Number(0)));

        let thread_impl = Self {
            shell,
            command_rx,
            cancel_rx,
            output_tx: output_tx.clone(),
            current_token,
            current_request_id,
            cols,
            rows,
        };

        let join_handle = tokio::spawn(async move {
            thread_impl.run().await;
        });

        let shell_handle = ShellHandle {
            command_tx,
            output_tx,
            cancel_tx,
        };

        Ok((shell_handle, join_handle))
    }

    async fn run(mut self) {
        log::debug!("KodegenInteractive task starting");

        loop {
            tokio::select! {
                biased;

                // Check for cancel signal when idle (to cancel current_token if set)
                maybe_cancel = self.cancel_rx.recv() => {
                    if maybe_cancel.is_some() {
                        log::info!("Received cancel signal while idle");
                        if let Ok(guard) = self.current_token.read()
                            && let Some(ref token) = *guard
                        {
                            token.cancel();
                            log::info!("Cancelled current execution token");
                        }
                    }
                }

                // Wait for command
                cmd = self.command_rx.recv() => {
                    match cmd {
                        Some(ExecuteCommand::Run { request_id, command }) => {
                            self.execute_command(request_id, command).await;
                        }
                        Some(ExecuteCommand::Shutdown) => {
                            log::debug!("KodegenInteractive: received Shutdown command, exiting");
                            break;
                        }
                        None => {
                            log::debug!("KodegenInteractive: channel closed, exiting");
                            break;
                        }
                    }
                }
            }
        }
        log::debug!("KodegenInteractive task exited cleanly");
    }

    async fn execute_command(
        &mut self,
        request_id: rmcp::model::RequestId,
        command: String,
    ) {
        log::debug!("execute_command: request_id={:?}, command={}", request_id, command);

        // Update current request_id
        if let Ok(mut guard) = self.current_request_id.write() {
            *guard = request_id.clone();
        } else {
            log::error!("Failed to acquire write lock on current_request_id - RwLock poisoned");
        }

        // 1. Create new cancellation token for this execution
        let token = CancellationToken::new();

        // 2. Store token so cancel_rx handler can access it
        if let Ok(mut guard) = self.current_token.write() {
            *guard = Some(token.clone());
        }

        // 3. Set up execution parameters with cancellation
        let mut params = self.shell.shell().default_exec_params();
        params.set_cancellation_token(token.clone());

        // 4. Start streaming execution
        let stream_result = self.shell.shell_mut().stream(&command, &params);
        
        let (mut stream, _stdin_tx) = match stream_result {
            Ok(s) => s,
            Err(e) => {
                log::error!("Failed to start command: {}", e);
                // Send error as output
                let _ = self.output_tx.send(ShellOutput::Bytes {
                    request_id: request_id.clone(),
                    data: format!("Error starting command: {}\n", e).into_bytes(),
                });
                // Send completion with error exit code
                let _ = self.output_tx.send(ShellOutput::ExecComplete {
                    request_id,
                    exit_code: 1,
                    cwd: self.shell.shell().working_dir().to_path_buf(),
                });
                return;
            }
        };

        log::debug!("Streaming command: {}", command);

        // 5. Process output stream with cancel check
        loop {
            tokio::select! {
                biased;

                // Check for cancel signal
                maybe_cancel = self.cancel_rx.recv() => {
                    if maybe_cancel.is_some() {
                        log::info!("Received cancel signal during command execution");
                        token.cancel();
                        // Stream will end naturally after cancellation
                    }
                }

                // Process stream output
                output = stream.next() => {
                    match output {
                        Some(chunk) => {
                            // Send output bytes via broadcast channel
                            let _ = self.output_tx.send(ShellOutput::Bytes {
                                request_id: request_id.clone(),
                                data: chunk.data,
                            });
                            
                            // Log stream type for debugging
                            match chunk.stream {
                                OutputStreamType::Stdout => log::trace!("stdout chunk"),
                                OutputStreamType::Stderr => log::trace!("stderr chunk"),
                            }
                        }
                        None => {
                            // Stream ended - command complete or cancelled
                            log::debug!("Stream ended");
                            break;
                        }
                    }
                }
            }
        }

        // 6. Clear current token
        if let Ok(mut guard) = self.current_token.write() {
            *guard = None;
        }

        // 7. Get exit code - if cancelled, use 130 (128 + SIGINT)
        let exit_code = if token.is_cancelled() {
            130u8
        } else {
            // Default to 0 if command completed without explicit exit code
            0u8
        };

        // Get current working directory from shell
        let current_cwd = self.shell.shell().working_dir().to_path_buf();

        // Update PS1 prompt
        let prompt = prmt::execute(
            "{path:#89dceb} {git:#f9e2af} {ok:#a6e3a1}{fail:#f38ba8} ",
            false,
            Some(exit_code as i32),
            false,
        ).unwrap_or_else(|_| "$ ".to_string());

        if let Err(e) = self.shell.shell_mut().env.set_global("PS1", ShellVariable::new(&prompt)) {
            log::warn!("Failed to set PS1: {}", e);
        }

        // Set COLUMNS and LINES for shell commands (ls, lsd, tree, etc.)
        if let Err(e) = self.shell.shell_mut().env.set_global("COLUMNS", ShellVariable::new(self.cols.to_string())) {
            log::warn!("Failed to set COLUMNS: {}", e);
        }
        if let Err(e) = self.shell.shell_mut().env.set_global("LINES", ShellVariable::new(self.rows.to_string())) {
            log::warn!("Failed to set LINES: {}", e);
        }

        // Send prompt as output bytes so it appears in terminal buffer
        let _ = self.output_tx.send(ShellOutput::Bytes {
            request_id: request_id.clone(),
            data: prompt.into_bytes(),
        });

        // Give a moment for output to be processed
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        // Send completion event
        let _ = self.output_tx.send(ShellOutput::ExecComplete {
            request_id,
            exit_code,
            cwd: current_cwd,
        });

        log::debug!("Command execution completed with exit_code={}", exit_code);
    }
}

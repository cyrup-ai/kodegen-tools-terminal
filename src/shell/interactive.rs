//! BrushInteractive thread - manages persistent shell and emits output events

use crate::pty::terminal::{ExecuteCommand, ShellOutput};
use crate::shell::BrushShell;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};

/// Handle to control the BrushInteractive thread
pub struct ShellHandle {
    pub command_tx: mpsc::Sender<ExecuteCommand>,
    pub output_tx: broadcast::Sender<ShellOutput>,
    pub shutdown_flag: Arc<AtomicBool>,
}

impl Drop for ShellHandle {
    fn drop(&mut self) {
        self.shutdown_flag.store(true, Ordering::Relaxed);
    }
}

/// BrushInteractive thread implementation
pub struct BrushInteractiveThread {
    shell: BrushShell,
    command_rx: mpsc::Receiver<ExecuteCommand>,
    output_tx: broadcast::Sender<ShellOutput>,
    shutdown_flag: Arc<AtomicBool>,
}

impl BrushInteractiveThread {
    pub fn spawn(shell: BrushShell) -> (ShellHandle, tokio::task::JoinHandle<()>) {
        let (command_tx, command_rx) = mpsc::channel(32);
        let (output_tx, _) = broadcast::channel(1024);
        let shutdown_flag = Arc::new(AtomicBool::new(false));

        let thread_impl = Self {
            shell,
            command_rx,
            output_tx: output_tx.clone(),
            shutdown_flag: shutdown_flag.clone(),
        };

        let join_handle = tokio::spawn(async move {
            thread_impl.run().await;
        });

        let shell_handle = ShellHandle {
            command_tx,
            output_tx,
            shutdown_flag,
        };

        (shell_handle, join_handle)
    }

    async fn run(mut self) {
        log::debug!("BrushInteractive task starting");

        while !self.shutdown_flag.load(Ordering::Relaxed) {
            // Use async recv instead of polling
            match self.command_rx.recv().await {
                Some(ExecuteCommand::Run {
                    request_id,
                    command,
                }) => {
                    self.execute_command(request_id, command).await;
                }
                None => {
                    log::debug!("BrushInteractive: channel closed, exiting");
                    break;
                }
            }
        }
        log::debug!("BrushInteractive task stopping");
    }

    async fn execute_command(
        &mut self,
        request_id: rmcp::model::RequestId,
        command: String,
    ) {
        log::debug!("execute_command: request_id={:?}, command={}", request_id, command);

        // Create per-command pipes for stdout and stderr (using std::io::pipe like brush)
        let (mut stdout_reader, stdout_writer) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to create stdout pipe: {}", e);
                return;
            }
        };

        let (mut stderr_reader, stderr_writer) = match std::io::pipe() {
            Ok(p) => p,
            Err(e) => {
                log::error!("Failed to create stderr pipe: {}", e);
                return;
            }
        };

        log::debug!("Created pipes");

        // Spawn tokio tasks to read pipes and broadcast output
        let output_tx_stdout = self.output_tx.clone();
        let request_id_stdout = request_id.clone();
        let _stdout_task = tokio::task::spawn_blocking(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match stdout_reader.read(&mut buffer) {
                    Ok(0) => {
                        log::debug!("stdout reader: EOF");
                        break;
                    }
                    Ok(n) => {
                        log::debug!("stdout reader: read {} bytes", n);
                        let _ = output_tx_stdout.send(ShellOutput::Bytes {
                            request_id: request_id_stdout.clone(),
                            data: buffer[..n].to_vec(),
                        });
                    }
                    Err(e) => {
                        log::error!("stdout reader error: {}", e);
                        break;
                    }
                }
            }
        });

        let output_tx_stderr = self.output_tx.clone();
        let request_id_stderr = request_id.clone();
        let _stderr_task = tokio::task::spawn_blocking(move || {
            let mut buffer = [0u8; 4096];
            loop {
                match stderr_reader.read(&mut buffer) {
                    Ok(0) => {
                        log::debug!("stderr reader: EOF");
                        break;
                    }
                    Ok(n) => {
                        log::debug!("stderr reader: read {} bytes", n);
                        let _ = output_tx_stderr.send(ShellOutput::Bytes {
                            request_id: request_id_stderr.clone(),
                            data: buffer[..n].to_vec(),
                        });
                    }
                    Err(e) => {
                        log::error!("stderr reader error: {}", e);
                        break;
                    }
                }
            }
        });

        log::debug!("Spawned pipe reader tasks");

        // Execute on persistent shell (don't clone - state must persist across commands)
        let mut params = self.shell.shell().default_exec_params();

        // Put writers into params
        params.set_fd(brush_core::openfiles::OpenFiles::STDOUT_FD, stdout_writer.into());
        params.set_fd(brush_core::openfiles::OpenFiles::STDERR_FD, stderr_writer.into());

        let cmd = command.clone();

        log::debug!("About to execute: {}", command);

        // Execute command directly (we're already in an async context)
        let result = match self.shell.shell_mut().run_string(&cmd, &params).await {
            Ok(exec_result) => exec_result,
            Err(e) => {
                log::error!("Command execution failed: {}", e);
                return;
            }
        };

        log::debug!("Command execution completed");

        let exit_code = result.exit_code.into();

        // Get current working directory after command execution
        let current_cwd = self.shell.shell().working_dir().to_path_buf();

        log::debug!("Sending completion event, exit_code={}", exit_code);

        // Send completion event with CWD immediately
        match self.output_tx.send(ShellOutput::ExecComplete {
            request_id: request_id.clone(),
            exit_code,
            cwd: current_cwd.clone(),
        }) {
            Ok(n) => log::debug!("ExecComplete event sent to {} receivers", n),
            Err(e) => log::error!("Failed to send ExecComplete event: {}", e),
        }

        // Note: stdout_task and stderr_task continue running for the lifetime of the shell
        // They're not per-command, they're per-shell session

    }
}

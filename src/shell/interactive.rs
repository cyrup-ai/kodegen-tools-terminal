//! BrushInteractive thread - manages persistent shell and emits output events

use crate::pty::terminal::{ExecuteCommand, ShellOutput};
use crate::shell::BrushShell;
use brush_core::variables::ShellVariable;
use rustix_openpty::openpty;
use rustix_openpty::rustix::termios::Winsize;
use std::io::Read;
use std::os::fd::AsRawFd;
use std::sync::{Arc, RwLock};
use tokio::sync::{broadcast, mpsc};

/// Handle to control the BrushInteractive thread
pub struct ShellHandle {
    pub command_tx: mpsc::Sender<ExecuteCommand>,
    pub output_tx: broadcast::Sender<ShellOutput>,
    pub pty_reader_join_handle: tokio::task::JoinHandle<()>,
}

/// BrushInteractive thread implementation
pub struct BrushInteractiveThread {
    shell: BrushShell,
    command_rx: mpsc::Receiver<ExecuteCommand>,
    output_tx: broadcast::Sender<ShellOutput>,
    current_request_id: Arc<RwLock<rmcp::model::RequestId>>,
}

impl BrushInteractiveThread {
    pub async fn spawn(cols: u16, rows: u16) -> Result<(ShellHandle, tokio::task::JoinHandle<()>), std::io::Error> {
        log::debug!("BrushInteractiveThread::spawn() called with cols={}, rows={}", cols, rows);
        
        // Create PTY pair with correct terminal dimensions
        let winsize = Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,  // Unused but required
            ws_ypixel: 0,  // Unused but required
        };

        let pty_result = openpty(None, Some(&winsize))
            .map_err(|e| std::io::Error::other(format!("Failed to create PTY: {}", e)))?;

        // PTY master - we read from this to get shell output
        let pty_master = pty_result.controller;  // OwnedFd
        let pty_slave = pty_result.user;         // OwnedFd

        // Set master to non-blocking for tokio
        unsafe {
            let flags = libc::fcntl(pty_master.as_raw_fd(), libc::F_GETFL, 0);
            libc::fcntl(pty_master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK);
        }

        // Clone PTY slave for stdin, stdout, and stderr
        let pty_slave_stdin = std::fs::File::from(pty_slave.try_clone()
            .map_err(|e| std::io::Error::other(format!("Failed to clone PTY slave for stdin: {}", e)))?);
        let pty_slave_stdout = std::fs::File::from(pty_slave.try_clone()
            .map_err(|e| std::io::Error::other(format!("Failed to clone PTY slave for stdout: {}", e)))?);
        let pty_slave_stderr = std::fs::File::from(pty_slave);

        // Wrap in OpenFile for Brush
        let stdin_openfile = brush_core::openfiles::OpenFile::File(pty_slave_stdin);
        let stdout_openfile = brush_core::openfiles::OpenFile::File(pty_slave_stdout);
        let stderr_openfile = brush_core::openfiles::OpenFile::File(pty_slave_stderr);

        // Create shell with PTY slave as stdin/stdout/stderr
        let shell = BrushShell::with_fds(stdin_openfile, stdout_openfile, stderr_openfile).await?;
        log::debug!("BrushShell created successfully with PTY");

        let (command_tx, command_rx) = mpsc::channel(32);
        let (output_tx, _) = broadcast::channel(1024);

        // Create shared request_id tracker (initialized with placeholder)
        let current_request_id = Arc::new(RwLock::new(rmcp::model::RequestId::Number(0)));

        // Spawn async PTY reader task
        let output_tx_reader = output_tx.clone();
        let mut shutdown_rx = output_tx.subscribe();
        let current_request_id_reader = current_request_id.clone();
        let pty_master_file = std::fs::File::from(pty_master);
        let pty_async = tokio::io::unix::AsyncFd::new(pty_master_file)
            .map_err(|e| std::io::Error::other(format!("Failed to create AsyncFd: {}", e)))?;
        
        let pty_reader_join_handle = tokio::spawn(async move {
            log::debug!("PTY reader task started");
            let mut buffer = [0u8; 4096];
            
            loop {
                tokio::select! {
                    // Check for shutdown event
                    result = shutdown_rx.recv() => {
                        match result {
                            Ok(ShellOutput::Shutdown) => {
                                log::debug!("PTY reader: received Shutdown event");
                                break;
                            }
                            Err(broadcast::error::RecvError::Closed) => {
                                log::debug!("PTY reader: channel closed");
                                break;
                            }
                            _ => continue, // Ignore other events
                        }
                    }
                    // Read from PTY master
                    result = pty_async.readable() => {
                        match result {
                            Ok(mut guard) => {
                                match guard.try_io(|inner| inner.get_ref().read(&mut buffer)) {
                                    Ok(Ok(0)) => {
                                        log::debug!("PTY reader: EOF");
                                        break;
                                    }
                                    Ok(Ok(n)) => {
                                        log::debug!("PTY reader: read {} bytes", n);
                                        let request_id = match current_request_id_reader.read() {
                                            Ok(g) => g.clone(),
                                            Err(e) => {
                                                log::error!("PTY reader: Failed to read current_request_id (poisoned): {}", e);
                                                rmcp::model::RequestId::Number(0)
                                            }
                                        };
                                        let _ = output_tx_reader.send(ShellOutput::Bytes {
                                            request_id,
                                            data: buffer[..n].to_vec(),
                                        });
                                    }
                                    Ok(Err(e)) => {
                                        log::error!("PTY reader error: {}", e);
                                        break;
                                    }
                                    Err(_would_block) => {
                                        // Spurious wakeup, continue
                                        continue;
                                    }
                                }
                            }
                            Err(e) => {
                                log::error!("PTY reader: readable error: {}", e);
                                break;
                            }
                        }
                    }
                }
            }
            log::debug!("PTY reader task exiting");
        });

        let thread_impl = Self {
            shell,
            command_rx,
            output_tx: output_tx.clone(),
            current_request_id,
        };

        let join_handle = tokio::spawn(async move {
            thread_impl.run().await;
        });

        let shell_handle = ShellHandle {
            command_tx,
            output_tx,
            pty_reader_join_handle,
        };

        Ok((shell_handle, join_handle))
    }

    async fn run(mut self) {
        log::debug!("BrushInteractive task starting");

        loop {
            match self.command_rx.recv().await {
                Some(ExecuteCommand::Run {
                    request_id,
                    command,
                }) => {
                    self.execute_command(request_id, command).await;
                }
                Some(ExecuteCommand::Shutdown) => {
                    log::debug!("BrushInteractive: received Shutdown command, exiting");
                    break;
                }
                None => {
                    log::debug!("BrushInteractive: channel closed, exiting");
                    break;
                }
            }
        }
        log::debug!("BrushInteractive task exited cleanly");
    }

    async fn execute_command(
        &mut self,
        request_id: rmcp::model::RequestId,
        command: String,
    ) {
        log::debug!("execute_command: request_id={:?}, command={}", request_id, command);

        // Update current request_id for PTY reader task
        if let Ok(mut guard) = self.current_request_id.write() {
            *guard = request_id.clone();
        } else {
            log::error!("Failed to acquire write lock on current_request_id - RwLock poisoned");
            // Still execute command, reader will use stale request_id
        }

        // Get default execution params (uses PTY slave FDs from shell)
        let params = self.shell.shell().default_exec_params();

        log::debug!("Executing command: {}", command);

        // Execute command on persistent shell (writes to PTY slave)
        let result = match self.shell.shell_mut().run_string(&command, &params).await {
            Ok(exec_result) => exec_result,
            Err(e) => {
                log::error!("Command execution failed: {}", e);
                return;
            }
        };

        log::debug!("Command completed");

        let exit_code = result.exit_code.into();

        // Get current working directory from shell
        let current_cwd = self.shell.shell().working_dir().to_path_buf();

        // Update PS1 prompt
        let prompt = prmt::execute(
            "{path:#89dceb} {git:#f9e2af} {ok:#a6e3a1}{fail:#f38ba8} ",
            false,
            Some(exit_code as i32),
            false,
        ).unwrap_or_else(|_| "$ ".to_string());

        let _ = self.shell.shell_mut().env.set_global("PS1", ShellVariable::new(&prompt));

        // Print the prompt to the terminal so it appears in the buffer
        {
            use std::io::Write;
            let shell = self.shell.shell();
            let mut stdout = shell.default_exec_params().stdout(shell);
            let _ = stdout.write_all(prompt.as_bytes());
            let _ = stdout.flush();
        }

        // Give PTY reader time to read the prompt bytes before we signal completion
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // Send completion event
        let _ = self.output_tx.send(ShellOutput::ExecComplete {
            request_id,
            exit_code,
            cwd: current_cwd,
        });

        log::debug!("Command execution completed");
    }
}

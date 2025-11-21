use std::path::PathBuf;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use parking_lot::Mutex as SyncMutex;
use alacritty_terminal::tty::{Options as PtyOptions, Shell, EventedReadWrite};
use alacritty_terminal::term::{Term as AlacrittyTerm, Config as AlacrittyConfig};
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Dimensions;
use vte::ansi::Processor;
use tokio::task;
use tokio::sync::{RwLock, mpsc};

use super::{shell::get_default_shell, types::{Terminal, HeadlessEventProxy}};

impl Terminal {
    /// Initialize the terminal with a command and spawn PTY I/O tasks
    ///
    /// # Architecture
    ///
    /// This creates:
    /// 1. An Alacritty Term<HeadlessEventProxy> for terminal emulation
    /// 2. A VTE Parser for ANSI/VT escape sequence parsing
    /// 3. A platform-specific PTY (Unix: rustix-openpty, Windows: ConPTY)
    /// 4. Two async tasks:
    ///    - Reader: PTY output → VTE parser → Term (updates grid)
    ///    - Writer: Input channel → PTY writer
    ///
    /// # Threading Model
    ///
    /// Uses `tokio::task::spawn_blocking` for PTY reading since EventedReadWrite
    /// provides synchronous I/O. This matches the current implementation pattern.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if initialization succeeds, or an error if PTY setup fails.
    pub async fn init(&mut self) -> io::Result<()> {
        // GUARD: Prevent double initialization
        if self.writer_task.is_some() || self.reader_task.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Terminal already initialized, cannot call init() again",
            ));
        }

        // 1. Build Alacritty Config for Term
        let alacritty_config = AlacrittyConfig {
            scrolling_history: self.config.scrollback,
            ..Default::default()
        };

        // 2. Create Term with HeadlessEventProxy
        let event_proxy = HeadlessEventProxy;
        let term = AlacrittyTerm::new(
            alacritty_config,
            &self.size,  // Implements Dimensions
            event_proxy,
        );

        self.term = Arc::new(RwLock::new(term));
        self.processor = Arc::new(RwLock::new(Processor::default()));

        // 3. Build PtyOptions from TerminalConfig
        let shell = if let Some(ref command) = self.config.command {
            if self.config.shell {
                // Run command through shell: shell_path -c "command"
                let default_shell = get_default_shell();
                let shell_exe = self.config.shell_path.as_deref().unwrap_or(&default_shell);
                Some(Shell::new(
                    shell_exe.to_string(),
                    vec!["-c".to_string(), command.clone()],
                ))
            } else {
                // Parse command and args (simple whitespace split)
                let parts: Vec<&str> = command.split_whitespace().collect();
                if parts.is_empty() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "Empty command provided",
                    ));
                }

                Some(Shell::new(
                    parts[0].to_string(),
                    parts[1..].iter().map(|s| s.to_string()).collect(),
                ))
            }
        } else if self.config.shell {
            // Just run shell with no command
            let default_shell = get_default_shell();
            let shell_exe = self.config.shell_path.as_deref().unwrap_or(&default_shell);
            Some(Shell::new(shell_exe.to_string(), vec![]))
        } else {
            // Default to shell if nothing specified
            let default_shell = get_default_shell();
            let shell_exe = self.config.shell_path.as_deref().unwrap_or(&default_shell);
            Some(Shell::new(shell_exe.to_string(), vec![]))
        };

        let pty_options = PtyOptions {
            shell,
            working_directory: self.config.cwd.as_ref().map(PathBuf::from),
            drain_on_exit: true,
            env: self.config.env_vars.clone(),

            #[cfg(target_os = "windows")]
            escape_args: true,
        };

        // 4. Create WindowSize for PTY
        let window_size = WindowSize {
            num_lines: self.size.rows,
            num_cols: self.size.cols,
            cell_width: 0,   // Pixel dimensions (not needed for headless)
            cell_height: 0,
        };

        // 5. Create platform-specific PTY using alacritty_terminal::tty::new()
        log::debug!("Creating PTY with command: {:?}", self.config.command);
        let pty = alacritty_terminal::tty::new(&pty_options, window_size, 0)
            .map_err(|e| io::Error::other(format!("Failed to create PTY: {e}")))?;
        log::info!("PTY created successfully");

        let pty_arc = Arc::new(SyncMutex::new(pty));
        self.pty = Some(pty_arc.clone());

        // 5.5. Create channel for PTY bytes (reader → processor)
        let (pty_bytes_tx, pty_bytes_rx) = mpsc::unbounded_channel();
        self.pty_bytes_tx = Some(pty_bytes_tx.clone());
        self.pty_bytes_rx = Some(pty_bytes_rx);

        // 6. Spawn reader task (blocking I/O only - sends raw bytes to processor)
        let pty_reader = pty_arc.clone();
        let pty_closed_flag = self.pty_closed.clone();

        let reader_handle = task::spawn_blocking(move || {
            log::info!("PTY reader task starting");
            
            // Direct lock acquisition using parking_lot
            let mut pty = pty_reader.lock();
            log::debug!("PTY reader: obtained PTY lock (holding for duration)");

            let mut buf = [0u8; 65536];  // 64KB buffer for better throughput

            loop {
                log::trace!("PTY reader: calling read()...");
                
                // Read from PTY using EventedReadWrite trait
                let size = match pty.reader().read(&mut buf) {
                    Ok(size) => size,
                    Err(e) => {
                        if e.kind() == io::ErrorKind::BrokenPipe {
                            log::info!("PTY reader: broken pipe (child exited)");
                            break;
                        } else if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted {
                            log::trace!("PTY read would block, retrying...");
                            continue;
                        } else {
                            log::error!("PTY read error: {e}");
                            break;
                        }
                    }
                };

                if size == 0 {
                    log::info!("PTY reader: EOF (0 bytes read)");
                    break;
                }

                log::debug!("PTY reader: read {} bytes from PTY", size);
                log::trace!("PTY bytes: {:?}", &buf[..size.min(100)]);

                // Send bytes to processor task via channel (NO processing here)
                if pty_bytes_tx.send(buf[..size].to_vec()).is_err() {
                    log::error!("PTY reader: processor task dropped channel");
                    break;
                }
            }

            pty_closed_flag.store(true, Ordering::SeqCst);
            log::info!("PTY reader task finished");
        });

        self.reader_task = Some(reader_handle);

        // 6.5. Spawn processor task (async - receives bytes and processes VTE)
        let processor_clone = self.processor.clone();
        let term_clone = self.term.clone();
        let mut pty_bytes_rx = self.pty_bytes_rx.take().ok_or_else(|| {
            // CLEANUP: Drop reader task before error
            self.reader_task = None;
            io::Error::other("pty_bytes_rx already taken")
        })?;

        let processor_handle = tokio::spawn(async move {
            log::info!("VTE processor task starting");

            while let Some(bytes) = pty_bytes_rx.recv().await {
                log::debug!("Processor: received {} bytes", bytes.len());

                // Acquire async locks with .await
                let mut processor = processor_clone.write().await;
                let mut term = term_clone.write().await;

                log::debug!("Processor: calling advance() with {} bytes", bytes.len());
                processor.advance(&mut *term, &bytes);
                log::debug!("Processor: advance() completed, grid now has {} screen lines",
                           term.grid().screen_lines());

                // Locks automatically dropped
            }

            log::info!("VTE processor task finished");
        });

        self.processor_task = Some(processor_handle);

        // 7. Spawn writer task (sends input to PTY)
        let pty_writer = pty_arc.clone();
        let mut rx = self.receiver.take().ok_or_else(|| {
            // CLEANUP: Drop reader and processor task handles before returning error
            self.reader_task = None;
            self.processor_task = None;
            io::Error::other("Receiver already taken")
        })?;

        let writer_handle = tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                // Clone Arc for move into spawn_blocking
                let pty_clone = pty_writer.clone();
                
                // Wrap blocking I/O in spawn_blocking
                let write_result = tokio::task::spawn_blocking(move || -> io::Result<()> {
                    // Direct lock acquisition using parking_lot
                    let mut pty = pty_clone.lock();
                    
                    // Blocking writes (safely in spawn_blocking)
                    pty.writer().write_all(&bytes)?;
                    pty.writer().flush()?;
                    
                    Ok(())
                }).await;
                
                // Handle spawn_blocking errors
                match write_result {
                    Ok(Ok(())) => {
                        // Write succeeded
                    }
                    Ok(Err(e)) => {
                        log::error!("PTY write error: {e}");
                        break;
                    }
                    Err(e) => {
                        log::error!("PTY write task panicked: {e}");
                        break;
                    }
                }
            }

            log::info!("PTY writer task finished");
        });

        self.writer_task = Some(writer_handle);

        Ok(())
    }
}

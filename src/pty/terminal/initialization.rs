use std::path::PathBuf;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use alacritty_terminal::tty::{Options as PtyOptions, Shell, EventedReadWrite};
use alacritty_terminal::term::{Term as AlacrittyTerm, Config as AlacrittyConfig};
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Dimensions;
use vte::ansi::Processor;
use tokio::{sync::Mutex, task};
use parking_lot::RwLock;

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

        let pty_arc = Arc::new(Mutex::new(pty));
        self.pty = Some(pty_arc.clone());

        // 6. Spawn reader task to process PTY output
        let term_clone = self.term.clone();
        let processor_clone = self.processor.clone();
        let pty_reader = pty_arc.clone();
        let pty_closed_flag = self.pty_closed.clone();

        let reader_handle = task::spawn_blocking(move || {
            log::info!("PTY reader task starting");
            
            // Get async runtime handle for blocking on async lock
            let rt = tokio::runtime::Handle::current();

            let mut pty = rt.block_on(pty_reader.lock());
            log::debug!("PTY reader: obtained PTY lock");

            let mut buf = [0u8; 65536];  // 64KB buffer for better throughput

            loop {
                log::trace!("PTY reader: calling read()...");
                
                // Unlock PTY during sleep to allow writer access
                drop(pty);
                std::thread::sleep(std::time::Duration::from_millis(10));
                pty = rt.block_on(pty_reader.lock());
                
                // Read from PTY using EventedReadWrite trait
                let size = match pty.reader().read(&mut buf) {
                    Ok(size) => size,
                    Err(e) => {
                        if e.kind() == io::ErrorKind::BrokenPipe {
                            log::info!("PTY reader: broken pipe (child exited)");
                            break;
                        } else if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted {
                            // Non-blocking PTY with no data yet, or interrupted syscall - retry
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
                    break;  // EOF
                }

                log::debug!("PTY reader: read {} bytes from PTY", size);
                log::trace!("PTY bytes: {:?}", &buf[..size.min(100)]);

                // Feed bytes through VTE processor → Term
                // CRITICAL: Use byte SLICE, not byte-by-byte iteration
                // This matches Alacritty's event_loop.rs:154 pattern
                let mut processor = processor_clone.write();
                let mut term = term_clone.write();

                log::debug!("PTY reader: calling processor.advance() with {} bytes", size);
                processor.advance(&mut *term, &buf[..size]);
                log::debug!("PTY reader: advance() completed, grid now has {} screen lines", 
                           term.grid().screen_lines());
            }

            pty_closed_flag.store(true, Ordering::SeqCst);
            log::info!("PTY output processing task finished");
        });

        self.reader_task = Some(reader_handle);

        // 7. Spawn writer task (sends input to PTY)
        let pty_writer = pty_arc.clone();
        let mut rx = self.receiver.take().ok_or_else(|| {
            // CLEANUP: Drop reader task handle before returning error
            self.reader_task = None;
            io::Error::other("Receiver already taken")
        })?;

        let writer_handle = tokio::spawn(async move {
            while let Some(bytes) = rx.recv().await {
                // Lock only for the write operation, then release
                let mut pty = pty_writer.lock().await;
                
                if let Err(e) = pty.writer().write_all(&bytes) {
                    log::error!("PTY write error: {e}");
                    break;
                }
                if let Err(e) = pty.writer().flush() {
                    log::error!("PTY flush error: {e}");
                    break;
                }
                
                // Lock is automatically released here when pty goes out of scope
            }

            log::info!("PTY writer task finished");
        });

        self.writer_task = Some(writer_handle);

        Ok(())
    }
}

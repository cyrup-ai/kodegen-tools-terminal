use std::{
    collections::HashMap,
    io,
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::broadcast;
use alacritty_terminal::term::{Term as AlacrittyTerm, Config as AlacrittyConfig};
use alacritty_terminal::tty::{Options as PtyOptions, Shell};
use alacritty_terminal::event::WindowSize;

use super::types::{TermSize, Terminal, TerminalConfig, HeadlessEventProxy};
use super::sync::FairMutex;
use super::shell::get_default_shell;
use super::event_loop::spawn_event_loop;

/// Builder for creating Terminal instances with a fluent API
#[derive(Default)]
pub struct TerminalBuilder {
    rows: Option<u16>,
    cols: Option<u16>,
    cwd: Option<String>,
    env_vars: HashMap<String, String>,
    shell_path: Option<String>,
    scrollback: usize,
}

impl TerminalBuilder {
    /// Create a new terminal builder with optimized defaults
    #[must_use]
    pub fn new() -> Self {
        Self {
            // Default to a comfortable terminal size
            rows: Some(30),
            cols: Some(100),
            cwd: None,
            env_vars: HashMap::from([
                // Enable truecolor support by default
                ("COLORTERM".to_string(), "truecolor".to_string()),
                // Ensure UTF-8 support
                ("LANG".to_string(), "en_US.UTF-8".to_string()),
                // Prefer more modern terminal features
                ("TERM".to_string(), "xterm-256color".to_string()),
            ]),
            shell_path: None,
            scrollback: 10000, // Generous scrollback by default
        }
    }

    /// Set terminal dimensions
    #[must_use]
    pub fn size(mut self, rows: u16, cols: u16) -> Self {
        self.rows = Some(rows);
        self.cols = Some(cols);
        self
    }

    /// Set working directory
    pub fn cwd(mut self, dir: impl Into<String>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Add environment variable
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_vars.insert(key.into(), value.into());
        self
    }

    /// Add multiple environment variables
    pub fn envs<K, V, I>(mut self, vars: I) -> Self
    where
        K: Into<String>,
        V: Into<String>,
        I: IntoIterator<Item = (K, V)>,
    {
        for (k, v) in vars {
            self.env_vars.insert(k.into(), v.into());
        }
        self
    }

    /// Specify which shell executable to use (overrides default detection)
    pub fn shell_path(mut self, path: impl Into<String>) -> Self {
        self.shell_path = Some(path.into());
        self
    }

    /// Set scrollback buffer size
    #[must_use]
    pub fn scrollback(mut self, lines: usize) -> Self {
        self.scrollback = lines;
        self
    }

    /// Build and initialize the terminal with all configured options
    ///
    /// Creates a fully initialized terminal with:
    /// - Alacritty Term for terminal emulation
    /// - VTE Processor for ANSI escape sequences
    /// - Platform-specific PTY (Unix/Windows)
    /// - Event loop thread for I/O
    ///
    /// Returns a ready-to-use Terminal (no separate init() needed).
    pub async fn build(self) -> io::Result<Terminal> {
        // Use sensible defaults for anything not specified
        let rows = self.rows.unwrap_or(30);
        let cols = self.cols.unwrap_or(100);

        let term_size = TermSize {
            cols,
            rows,
            scrollback: self.scrollback,
        };

        // Create configuration from builder settings
        let config = TerminalConfig {
            cwd: self.cwd.clone(),
            env_vars: self.env_vars.clone(),
            shell_path: self.shell_path.clone(),
            scrollback: self.scrollback,
        };

        // 1. Build Alacritty Config for Term
        let alacritty_config = AlacrittyConfig {
            scrolling_history: self.scrollback,
            ..Default::default()
        };

        // 2. Create Term with HeadlessEventProxy (wrapped in FairMutex)
        let event_proxy = HeadlessEventProxy;
        let term = AlacrittyTerm::new(
            alacritty_config,
            &term_size,
            event_proxy,
        );

        let term = Arc::new(FairMutex::new(term));

        // 3. Build PtyOptions from TerminalConfig
        // ALWAYS start interactive shell (no -c, stays open for follow-up commands!)
        let default_shell = get_default_shell();
        let shell_exe = self.shell_path.as_deref().unwrap_or(&default_shell);
        let shell = Some(Shell::new(shell_exe.to_string(), vec![]));

        let pty_options = PtyOptions {
            shell,
            working_directory: self.cwd.as_ref().map(PathBuf::from),
            drain_on_exit: true,
            env: self.env_vars.clone(),

            #[cfg(target_os = "windows")]
            escape_args: true,
        };

        // 4. Create WindowSize for PTY
        let window_size = WindowSize {
            num_lines: rows,
            num_cols: cols,
            cell_width: 0,   // Pixel dimensions (not needed for headless)
            cell_height: 0,
        };

        // 5. Create platform-specific PTY using alacritty_terminal::tty::new()
        // This automatically selects Unix PTY or Windows ConPTY at compile time
        log::debug!("Creating interactive PTY shell");
        let pty = alacritty_terminal::tty::new(&pty_options, window_size, 0)
            .map_err(|e| io::Error::other(format!("Failed to create PTY: {e}")))?;
        log::info!("PTY created successfully");

        // 6. Capture child PID BEFORE moving PTY (PID is immutable after spawn)
        let child_pid = pty.child().id();
        log::debug!("Captured child PID: {}", child_pid);

        // 7. Create broadcast channels for screen and bell notifications (capacity: 1000)
        let (output_broadcast, _) = broadcast::channel::<()>(1000);
        let output_broadcast = Arc::new(output_broadcast);

        let (bell_broadcast, _) = broadcast::channel::<()>(1000);
        let bell_broadcast = Arc::new(bell_broadcast);

        // 8. Move PTY into event loop (returns handle + InputSender)
        // The generic spawn_event_loop function works with any type implementing EventedPty
        let pty_closed = Arc::new(AtomicBool::new(false));
        log::info!("Spawning PTY event loop thread with direct PTY ownership");
        let (event_loop_handle, input_sender) = spawn_event_loop(
            pty,  // PTY moved here, no longer accessible
            term.clone(),
            output_broadcast.clone(),
            bell_broadcast.clone(),
            pty_closed.clone(),
        )?;

        log::info!("Terminal initialized with perfect event loop architecture + bell detection");

        Ok(Terminal {
            term,
            sender: Some(input_sender),
            size: term_size,
            pty_closed,
            config,
            event_loop_thread: Some(event_loop_handle),
            child_pid: Some(child_pid),
            output_broadcast,
            bell_broadcast,
        })
    }
}

use std::{
    collections::HashMap,
    io,
};

use super::types::{TermSize, Terminal};

/// Builder for creating Terminal instances with a fluent API
#[derive(Default)]
pub struct TerminalBuilder {
    terminal_id: Option<u32>,
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
            terminal_id: None,
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

    /// Set terminal ID
    #[must_use]
    pub fn terminal_id(mut self, id: u32) -> Self {
        self.terminal_id = Some(id);
        self
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
    /// Creates a fully initialized terminal with three-thread architecture:
    /// - BrushExecutor: Executes commands, emits ShellOutput events
    /// - VteProcessor: Processes VTE sequences, maintains terminal grid, emits TerminalBuffer events
    /// - TerminalManager: API layer (subscribes to TerminalBuffer events)
    ///
    /// Returns a ready-to-use Terminal (no separate init() needed).
    pub async fn build(self) -> io::Result<Terminal> {
        log::debug!("TerminalBuilder::build() called");
        let rows = self.rows.unwrap_or(200);
        let cols = self.cols.unwrap_or(120);

        let term_size = TermSize {
            cols,
            rows,
            scrollback: self.scrollback,
        };

        // Spawn KodegenInteractive thread (creates shell with streaming + cancellation)
        let (shell_handle, shell_join_handle) = crate::shell::KodegenInteractiveThread::spawn(cols, rows).await?;

        // Subscribe to shell output for VTE processing
        let shell_output_rx = shell_handle.output_tx.subscribe();

        // Get initial CWD for VteProcessor
        let initial_cwd = self.cwd
            .or_else(|| std::env::current_dir().ok().and_then(|p| p.to_str().map(String::from)))
            .unwrap_or_else(|| "/".to_string());
        let initial_cwd = std::path::PathBuf::from(initial_cwd);

        // Spawn VteProcessor thread (creates its own Term with term_size)
        let (vte_handle, vte_join_handle) = super::VteProcessorThread::spawn(
            shell_output_rx,
            initial_cwd,
            term_size,
        );

        // Subscribe to TerminalBuffer events BEFORE any events can be emitted
        let buffer_rx = vte_handle.buffer_tx.subscribe();

        // NEW: Create ValidationEngine with default rules
        let validation_engine = crate::validation::ValidationEngine::new();
        crate::validation::register_default_rules(&validation_engine);

        // CommandManager provides parsing utilities only (validation is done by ValidationEngine)
        let command_manager = crate::validation::CommandManager::new();

        log::info!("Terminal initialized with streaming + cancellation architecture (KodegenShell + VteProcessor)");
        log::info!("ValidationEngine initialized with default security rules");

        Ok(Terminal {
            terminal_id: self.terminal_id.unwrap_or(0),
            shell_handle: Some(shell_handle),
            shell_join_handle: Some(shell_join_handle),
            vte_handle: Some(vte_handle),
            vte_join_handle: Some(vte_join_handle),
            buffer_rx: tokio::sync::Mutex::new(buffer_rx),
            validation_engine,
            command_manager,
        })
    }
}

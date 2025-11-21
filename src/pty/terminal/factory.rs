use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
};
use tokio::sync::mpsc::channel;
use tokio::sync::RwLock;
use vte::ansi::Processor;
use alacritty_terminal::term::{Term as AlacrittyTerm, Config as AlacrittyConfig};

use super::{
    builder::TerminalBuilder,
    types::{BellStyle, ColorMode, TermSize, Terminal, TerminalConfig, HeadlessEventProxy},
};

impl Terminal {
    /// Create a convenient builder for terminal creation
    #[must_use]
    pub fn builder() -> TerminalBuilder {
        TerminalBuilder::new()
    }

    /// Quick terminal for running a single command
    pub fn quick(command: impl Into<String>) -> Self {
        Self::builder().command(command).exit_on_close(true).build()
    }

    /// Interactive shell optimized for development
    #[must_use]
    pub fn dev_shell() -> Self {
        Self::builder()
            .shell(true)
            .env(
                "PS1",
                "\\[\\033[01;32m\\]\\u@\\h\\[\\033[00m\\]:\\[\\033[01;34m\\]\\w\\[\\033[00m\\]\\$ ",
            )
            .env(
                "PATH",
                format!(
                    "{}:{}",
                    std::env::var("PATH").unwrap_or_default(),
                    "/usr/local/bin"
                ),
            )
            .exit_on_close(false)
            .build()
    }

    /// Terminal optimized for running JS/TS dev servers
    #[must_use]
    pub fn node_dev() -> Self {
        Self::builder()
            .shell(true)
            .env("NODE_ENV", "development")
            .env("DEBUG", "*")
            .force_color(true)
            .build()
    }

    /// Terminal optimized for Rust development
    #[must_use]
    pub fn rust_dev() -> Self {
        Self::builder()
            .shell(true)
            .env("RUST_BACKTRACE", "1")
            .env("RUSTFLAGS", "-C debug-assertions=yes")
            .force_color(true)
            .build()
    }

    /// Create a new terminal with the given size
    #[must_use]
    pub fn new(size: TermSize) -> Self {
        let (tx, rx) = channel(32);

        // Create default config
        let config = TerminalConfig {
            command: None,
            cwd: None,
            env_vars: HashMap::new(),
            shell: true,
            shell_path: None,
            colors: ColorMode::TrueColor,
            scrollback: 0,
            cursor_blink: true,
            application_cursor_keys: true,
            bracketed_paste: true,
            mouse_reporting: true,
            alternate_screen: false,
            exit_on_close: true,
            bell_style: BellStyle::Visual,
        };

        // Create Alacritty Term with HeadlessEventProxy
        let alacritty_config = AlacrittyConfig {
            scrolling_history: 0,
            ..Default::default()
        };
        let event_proxy = HeadlessEventProxy;
        let term = AlacrittyTerm::new(alacritty_config, &size, event_proxy);

        // Create VTE processor
        let processor = Arc::new(RwLock::new(Processor::default()));
        let term = Arc::new(RwLock::new(term));

        Self {
            term,
            processor,
            sender: Some(tx),
            receiver: Some(rx),
            size,
            pty_closed: Arc::new(AtomicBool::new(false)),
            config,
            pty: None,
            pty_bytes_tx: None,
            pty_bytes_rx: None,
            reader_task: None,
            processor_task: None,
            writer_task: None,
        }
    }
}

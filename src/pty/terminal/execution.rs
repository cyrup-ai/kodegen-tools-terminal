use std::io;
use bytes::Bytes;

use super::types::{Terminal, KeyCode};

impl Terminal {
    /// Execute a command in the terminal and return the resulting screen contents
    ///
    /// Returns a `JoinHandle` that resolves to the final screen contents when the command completes.
    ///
    /// # Errors
    /// Returns error if:
    /// - Terminal already initialized (call `exec()` only once per Terminal)
    /// - PTY creation fails
    /// - Command spawn fails
    /// - System resource limits reached
    ///
    /// # Example
    /// ```no_run
    /// # use kodegen_tools_terminal::pty::Terminal;
    /// # use std::error::Error;
    /// # async fn example() -> Result<(), Box<dyn Error>> {
    /// let mut term = Terminal::builder().build();
    /// let handle = term.exec("ls -la").await?;
    /// let output = handle.await?;
    /// println!("{}", output);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn exec(
        &mut self,
        command: impl Into<String> + Send + 'static,
    ) -> io::Result<tokio::task::JoinHandle<String>> {
        let command_str = command.into();

        // GUARD: Prevent double initialization (check if tasks are already running)
        if self.writer_task.is_some() || self.reader_task.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "Terminal already initialized, cannot call exec() again",
            ));
        }

        // Update config to run this command through shell
        self.config.command = Some(command_str);
        self.config.shell = true;

        // Initialize the terminal with the configured command
        self.init().await?;

        // Clone self to move into the async task
        let terminal = self.clone();

        // Spawn an async task that waits for the command to complete
        Ok(tokio::spawn(async move {
            // Poll until the PTY is closed (command finished)
            while !terminal.is_pty_closed() {
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }

            // Try to get the screen, retrying a few times if the lock is busy
            for attempt in 0..10 {
                if let Some(screen) = terminal.screen() {
                    return screen;
                }
                if attempt < 9 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                }
            }

            // If we still can't get the screen after retries, return empty string
            // This should be extremely rare (only if the lock fails)
            log::error!("Failed to acquire screen after retries, returning empty string");
            String::new()
        }))
    }

    /// Send input bytes to the terminal
    pub async fn send_input(&self, bytes: Bytes) -> io::Result<()> {
        let sender = self
            .sender
            .as_ref()
            .ok_or_else(|| io::Error::other("Terminal has been closed"))?;
        sender
            .send(bytes)
            .await
            .map_err(|e| io::Error::other(format!("Failed to send input: {e}")))
    }

    /// Send a character to the terminal
    pub async fn send_char(&self, c: char) -> io::Result<()> {
        self.send_input(Bytes::from(c.to_string().into_bytes()))
            .await
    }

    /// Send a key code to the terminal (special keys like arrows, backspace, etc.)
    pub async fn send_keycode(&self, code: KeyCode) -> io::Result<()> {
        let bytes = match code {
            KeyCode::Backspace => Bytes::from(vec![8]),
            KeyCode::Enter => Bytes::from(vec![b'\n']),
            KeyCode::Left => Bytes::from(vec![27, 91, 68]),
            KeyCode::Right => Bytes::from(vec![27, 91, 67]),
            KeyCode::Up => Bytes::from(vec![27, 91, 65]),
            KeyCode::Down => Bytes::from(vec![27, 91, 66]),
            KeyCode::Tab => Bytes::from(vec![9]),
            KeyCode::Delete => Bytes::from(vec![27, 91, 51, 126]),
            KeyCode::Home => Bytes::from(vec![27, 79, 72]),
            KeyCode::End => Bytes::from(vec![27, 79, 70]),
            KeyCode::PageUp => Bytes::from(vec![27, 91, 53, 126]),
            KeyCode::PageDown => Bytes::from(vec![27, 91, 54, 126]),
            KeyCode::Esc => Bytes::from(vec![27]),
        };

        self.send_input(bytes).await
    }
}

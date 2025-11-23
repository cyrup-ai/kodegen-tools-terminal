use std::io;

use super::types::{Terminal, KeyCode};

impl Terminal {
    /// Send input bytes to the terminal
    pub async fn send_input(&self, bytes: Vec<u8>) -> io::Result<()> {
        let sender = self
            .sender
            .clone()
            .ok_or_else(|| io::Error::other("Terminal has been closed"))?;

        // InputSender::send is synchronous, so use spawn_blocking
        tokio::task::spawn_blocking(move || {
            sender.send(bytes)  // Returns io::Result, includes poller.notify()
        })
        .await
        .map_err(|e| io::Error::other(format!("Task join error: {e}")))?
    }

    /// Send a character to the terminal
    pub async fn send_char(&self, c: char) -> io::Result<()> {
        self.send_input(c.to_string().into_bytes())
            .await
    }

    /// Send a key code to the terminal (special keys like arrows, backspace, etc.)
    pub async fn send_keycode(&self, code: KeyCode) -> io::Result<()> {
        let bytes = match code {
            KeyCode::Backspace => vec![8],
            KeyCode::Enter => vec![b'\n'],
            KeyCode::Left => vec![27, 91, 68],
            KeyCode::Right => vec![27, 91, 67],
            KeyCode::Up => vec![27, 91, 65],
            KeyCode::Down => vec![27, 91, 66],
            KeyCode::Tab => vec![9],
            KeyCode::Delete => vec![27, 91, 51, 126],
            KeyCode::Home => vec![27, 79, 72],
            KeyCode::End => vec![27, 79, 70],
            KeyCode::PageUp => vec![27, 91, 53, 126],
            KeyCode::PageDown => vec![27, 91, 54, 126],
            KeyCode::Esc => vec![27],
        };

        self.send_input(bytes).await
    }

    /// Get the child process ID (PID) of the terminal
    ///
    /// Returns the PID of the child process spawned by the PTY.
    /// The PID is captured during initialization and remains valid for the lifetime
    /// of the process.
    ///
    /// Returns None if the terminal has not been initialized yet.
    pub fn try_get_pid(&self) -> Option<u32> {
        self.child_pid
    }
}

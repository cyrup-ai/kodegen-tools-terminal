//! EventBridge - EventListener implementation for VteProcessor's Term

use crate::pty::terminal::events::TerminalBuffer;
use alacritty_terminal::event::{Event, EventListener};
use tokio::sync::broadcast;

/// Event bridge between Alacritty Term and TerminalBuffer broadcast channel
///
/// Implements EventListener trait to receive events from Term during VTE processing.
/// Forwards relevant events (Title) as TerminalBuffer events.
pub struct EventBridge {
    buffer_tx: broadcast::Sender<TerminalBuffer>,
}

impl EventBridge {
    pub fn new(buffer_tx: broadcast::Sender<TerminalBuffer>) -> Self {
        Self { buffer_tx }
    }
}

impl EventListener for EventBridge {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(title) => {
                let _ = self.buffer_tx.send(TerminalBuffer::TitleChanged { title });
            }
            Event::Wakeup => {
                // VteProcessor extracts buffer after parser.advance()
            }
            _ => {
                // Ignore other events (Bell, MouseCursorDirty, etc.)
            }
        }
    }
}

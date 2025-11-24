//! Terminal registry - manages multiple terminal instances

use crate::pty::terminal::Terminal;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

type TerminalMap = HashMap<(String, u32), Arc<Terminal>>;

/// Registry for managing multiple terminal instances keyed by (connection_id, terminal_id)
#[derive(Clone)]
pub struct TerminalRegistry {
    terminals: Arc<Mutex<TerminalMap>>,
}

impl TerminalRegistry {
    /// Create a new terminal registry
    pub fn new() -> Self {
        Self {
            terminals: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Find or create a terminal instance
    pub async fn find_or_create_terminal(
        &self,
        connection_id: &str,
        terminal_id: u32,
    ) -> Result<Arc<Terminal>, anyhow::Error> {
        let key = (connection_id.to_string(), terminal_id);
        let mut terminals = self.terminals.lock().await;

        if let Some(terminal) = terminals.get(&key) {
            return Ok(terminal.clone());
        }

        let terminal = Arc::new(
            Terminal::builder()
                .terminal_id(terminal_id)
                .size(24, 80)
                .build()
                .await?,
        );

        terminals.insert(key, terminal.clone());
        Ok(terminal)
    }
}

impl Default for TerminalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

//! Terminal registry - manages multiple terminal instances

use crate::pty::terminal::Terminal;
use kodegen_mcp_schema::terminal::TerminalOutput;
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

    /// List all active terminals for a connection with their current states
    pub async fn list_all_terminals(
        &self,
        connection_id: &str,
    ) -> Result<TerminalOutput, anyhow::Error> {
        let start = std::time::Instant::now();
        let terminals = self.terminals.lock().await;

        let mut snapshots = Vec::new();
        for ((conn_id, term_id), terminal) in terminals.iter() {
            if conn_id == connection_id {
                // Get current state without blocking
                let state = terminal.read_current_state(*term_id).await?;
                snapshots.push(serde_json::json!({
                    "terminal": term_id,
                    "output": state.output,
                    "cwd": state.cwd,
                    "exit_code": state.exit_code,
                    "completed": state.completed,
                }));
            }
        }

        // Sort by terminal ID
        snapshots.sort_by_key(|v| v["terminal"].as_u64().unwrap_or(0));

        Ok(TerminalOutput {
            terminal: None, // None indicates LIST response with multiple terminals
            output: serde_json::to_string_pretty(&snapshots)?,
            exit_code: Some(0),
            cwd: "/".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            completed: true,
        })
    }

    /// Kill a terminal and cleanup all resources
    pub async fn kill_terminal(
        &self,
        connection_id: &str,
        terminal_id: u32,
    ) -> Result<TerminalOutput, anyhow::Error> {
        let start = std::time::Instant::now();
        let key = (connection_id.to_string(), terminal_id);
        let mut terminals = self.terminals.lock().await;

        if let Some(terminal) = terminals.remove(&key) {
            // Terminal::drop() handles graceful shutdown of all components
            drop(terminal);

            Ok(TerminalOutput {
                terminal: Some(terminal_id),
                output: format!(
                    "Terminal {} gracefully shutdown and all resources cleaned up",
                    terminal_id
                ),
                exit_code: Some(0),
                cwd: "/".to_string(),
                duration_ms: start.elapsed().as_millis() as u64,
                completed: true,
            })
        } else {
            Err(anyhow::anyhow!(
                "Terminal {} not found for connection {}",
                terminal_id,
                connection_id
            ))
        }
    }

    /// Cleanup all terminals for a connection
    ///
    /// Called when a connection drops to cleanup all associated terminal sessions.
    /// Returns the number of terminals cleaned up.
    pub async fn cleanup_connection(&self, connection_id: &str) -> usize {
        let mut terminals = self.terminals.lock().await;

        // Collect terminal IDs to remove
        let to_remove: Vec<(String, u32)> = terminals
            .keys()
            .filter(|(conn_id, _)| conn_id == connection_id)
            .cloned()
            .collect();

        let count = to_remove.len();

        // Remove and drop each terminal (Drop impl kills shell, closes PTY)
        for key in to_remove {
            if let Some(terminal) = terminals.remove(&key) {
                log::debug!(
                    "Cleaning up terminal {} for connection {}",
                    key.1,
                    connection_id
                );
                drop(terminal);
            }
        }

        count
    }
}

impl Default for TerminalRegistry {
    fn default() -> Self {
        Self::new()
    }
}

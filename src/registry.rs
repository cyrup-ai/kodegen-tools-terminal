//! Terminal registry - manages multiple terminal instances

use crate::pty::terminal::{Terminal, types::TerminalCommandResult};
use kodegen_mcp_schema::terminal::TerminalSnapshot;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Terminal lifecycle state
#[derive(Clone)]
enum TerminalState {
    /// Terminal is active and can be used for commands
    Active(Arc<Terminal>),
    
    /// Terminal is shutting down asynchronously
    /// This tombstone prevents recreation while cleanup is in progress.
    /// Tombstone is removed after shutdown completes.
    ShuttingDown,
}

type TerminalMap = HashMap<(String, u32), TerminalState>;

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
        pwd: Option<String>,
    ) -> Result<Arc<Terminal>, anyhow::Error> {
        let key = (connection_id.to_string(), terminal_id);
        let mut terminals = self.terminals.lock().await;

        match terminals.get(&key) {
            Some(TerminalState::Active(terminal)) => {
                // Terminal exists and is active
                return Ok(terminal.clone());
            }
            Some(TerminalState::ShuttingDown) => {
                // Terminal is shutting down, cannot be reused
                return Err(anyhow::anyhow!(
                    "Terminal {} is shutting down, please wait or use a different terminal ID",
                    terminal_id
                ));
            }
            None => {
                // Terminal doesn't exist, create new one
            }
        }

        // Create new terminal
        let mut builder = Terminal::builder()
            .terminal_id(terminal_id)
            .size(2000, 120);  // Force dimensions: 2000 rows x 120 cols

        if let Some(cwd) = pwd {
            builder = builder.cwd(cwd);
        }

        let terminal = Arc::new(builder.build().await?);

        terminals.insert(key, TerminalState::Active(terminal.clone()));
        Ok(terminal)
    }

    /// List all active terminals for a connection with their current states
    pub async fn list_all_terminals(
        &self,
        connection_id: &str,
    ) -> Result<TerminalCommandResult, anyhow::Error> {
        let start = std::time::Instant::now();
        let terminals = self.terminals.lock().await;

        let mut snapshots = Vec::new();
        for ((conn_id, term_id), state) in terminals.iter() {
            if conn_id == connection_id {
                // Only include Active terminals in list
                if let TerminalState::Active(terminal) = state {
                    let state = terminal.read_current_state(*term_id, 2000).await?;
                    snapshots.push(TerminalSnapshot {
                        terminal: *term_id,
                        cwd: state.cwd,
                        exit_code: state.exit_code,
                        completed: state.completed,
                    });
                }
                // Skip ShuttingDown tombstones
            }
        }

        // Sort by terminal ID
        snapshots.sort_by_key(|s| s.terminal);

        let output = serde_json::to_string_pretty(&snapshots)?;

        Ok(TerminalCommandResult {
            terminal: None,
            output,
            exit_code: Some(0),
            cwd: "/".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            completed: true,
            terminals: snapshots,
        })
    }

    /// Kill a terminal and cleanup all resources
    pub async fn kill_terminal(
        &self,
        connection_id: &str,
        terminal_id: u32,
    ) -> Result<TerminalCommandResult, anyhow::Error> {
        let start = std::time::Instant::now();
        let key = (connection_id.to_string(), terminal_id);
        
        // Phase 1: Replace terminal with tombstone atomically
        let terminal_arc = {
            let mut terminals = self.terminals.lock().await;
            
            match terminals.get(&key) {
                Some(TerminalState::Active(terminal)) => {
                    let terminal = terminal.clone();
                    
                    // Replace Active with ShuttingDown tombstone BEFORE dropping lock
                    // This prevents recreation during shutdown
                    terminals.insert(key.clone(), TerminalState::ShuttingDown);
                    
                    Some(terminal)
                }
                Some(TerminalState::ShuttingDown) => {
                    // Already shutting down
                    return Err(anyhow::anyhow!(
                        "Terminal {} is already shutting down",
                        terminal_id
                    ));
                }
                None => {
                    // Terminal not found
                    return Err(anyhow::anyhow!(
                        "Terminal {} not found for connection {}",
                        terminal_id,
                        connection_id
                    ));
                }
            }
        }; // Lock dropped here, but tombstone prevents recreation
        
        // Phase 2: Shutdown terminal (lock is released, tombstone prevents recreation)
        if let Some(terminal_arc) = terminal_arc {
            match Arc::try_unwrap(terminal_arc) {
                Ok(terminal) => {
                    terminal.shutdown().await;
                }
                Err(arc) => {
                    // Arc still has references, but we can force shutdown via signals
                    let ref_count = Arc::strong_count(&arc);
                    log::warn!(
                        "Terminal {} still has {} references, forcing shutdown via signals",
                        terminal_id,
                        ref_count
                    );
                    
                    // Send shutdown signals (threads will exit and cleanup via RAII)
                    if let Err(e) = arc.force_shutdown_signals().await {
                        log::error!("Failed to send shutdown signals for terminal {}: {}", terminal_id, e);
                    } else {
                        log::info!(
                            "Terminal {} shutdown signals sent ({} Arc references remain, cleanup will be async)",
                            terminal_id,
                            ref_count
                        );
                    }
                }
            }
        }
        
        // Phase 3: Remove tombstone after shutdown completes
        {
            let mut terminals = self.terminals.lock().await;
            terminals.remove(&key);
        }

        Ok(TerminalCommandResult {
            terminal: Some(terminal_id),
            output: format!("Terminal {} shutdown complete", terminal_id),
            exit_code: Some(0),
            cwd: "/".to_string(),
            duration_ms: start.elapsed().as_millis() as u64,
            completed: true,
            terminals: Vec::new(),
        })
    }

    /// Cleanup all terminals for a connection
    ///
    /// Called when a connection drops to cleanup all associated terminal sessions.
    /// Returns the number of terminals cleaned up.
    pub async fn cleanup_connection(&self, connection_id: &str) -> usize {
        // Phase 1: Collect active terminals and mark as ShuttingDown
        let terminal_arcs = {
            let mut terminals = self.terminals.lock().await;

            // Find all terminals for this connection
            let keys_to_cleanup: Vec<(String, u32)> = terminals
                .keys()
                .filter(|(conn_id, _)| conn_id == connection_id)
                .cloned()
                .collect();

            // Extract Active terminals and replace with ShuttingDown tombstones
            let mut extracted = Vec::new();
            for key in &keys_to_cleanup {
                if let Some(TerminalState::Active(terminal)) = terminals.get(key) {
                    let terminal = terminal.clone();
                    terminals.insert(key.clone(), TerminalState::ShuttingDown);
                    extracted.push((key.clone(), terminal));
                }
                // Skip already ShuttingDown tombstones
            }

            extracted
        }; // Lock dropped, tombstones prevent recreation

        let count = terminal_arcs.len();

        // Phase 2: Shutdown each terminal (outside lock)
        for (_key, terminal_arc) in terminal_arcs {
            match Arc::try_unwrap(terminal_arc) {
                Ok(terminal) => {
                    log::debug!("Shutting down terminal for connection {}", connection_id);
                    terminal.shutdown().await;
                }
                Err(arc) => {
                    // Arc still has references, but we can force shutdown via signals
                    let ref_count = Arc::strong_count(&arc);
                    log::warn!(
                        "Terminal for connection {} still has {} references, forcing shutdown via signals",
                        connection_id,
                        ref_count
                    );
                    
                    // Send shutdown signals (threads will exit and cleanup via RAII)
                    if let Err(e) = arc.force_shutdown_signals().await {
                        log::error!(
                            "Failed to send shutdown signals for connection {}: {}",
                            connection_id,
                            e
                        );
                    } else {
                        log::debug!(
                            "Terminal shutdown signals sent for connection {} ({} Arc references remain)",
                            connection_id,
                            ref_count
                        );
                    }
                }
            }
        }

        // Phase 3: Remove all tombstones for this connection
        {
            let mut terminals = self.terminals.lock().await;
            
            let keys_to_remove: Vec<(String, u32)> = terminals
                .keys()
                .filter(|(conn_id, _)| conn_id == connection_id)
                .cloned()
                .collect();

            for key in keys_to_remove {
                terminals.remove(&key);
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

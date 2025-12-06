//! Terminal tool implementation

use crate::TerminalRegistry;
use kodegen_mcp_schema::terminal::{TERMINAL, TerminalAction, TerminalInput, TerminalOutput, TerminalPrompts};
use kodegen_mcp_schema::McpError;
use kodegen_mcp_schema::{Tool, ToolExecutionContext, ToolResponse};
use std::sync::Arc;

/// Terminal tool - executes commands in persistent terminal sessions
///
/// This is a thin wrapper around TerminalRegistry that provides the MCP tool interface.
/// TerminalRegistry manages multiple terminals, each built with proper lifespans.
#[derive(Clone)]
pub struct TerminalTool {
    registry: Arc<TerminalRegistry>,
}

impl TerminalTool {
    /// Create a new terminal tool instance
    pub fn new() -> Self {
        Self {
            registry: Arc::new(TerminalRegistry::new()),
        }
    }

    /// Get the terminal registry for connection cleanup
    pub fn registry(&self) -> Arc<TerminalRegistry> {
        self.registry.clone()
    }
}

impl Default for TerminalTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for TerminalTool {
    type Args = TerminalInput;
    type Prompts = TerminalPrompts;

    fn name() -> &'static str {
        TERMINAL
    }

    fn description() -> &'static str {
        "Execute shell commands in persistent, stateful terminal sessions with multiplatform support. \
         Supports 4 actions: EXEC (execute command), READ (get current buffer), LIST (show all terminals), \
         KILL (gracefully shutdown). Terminals maintain environment variables, working directory, and \
         shell state across commands. Use different terminal numbers (0, 1, 2...) for parallel work. \
         Returns complete terminal output - actual rendered terminal output, not raw bytes. \
         Supports background tasks (await_completion_ms=0) and timeout with continuation."
    }

    fn read_only() -> bool {
        false
    }

    fn destructive() -> bool {
        true
    }

    fn idempotent() -> bool {
        false
    }

    fn open_world() -> bool {
        true
    }

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<ToolResponse<TerminalOutput>, McpError> {
        let start = std::time::Instant::now();
        let connection_id = ctx.connection_id().unwrap_or("default");
        let request_id = ctx.request_id().clone();
        let terminal_id = args.terminal;

        // Dispatch based on action
        let output = match args.action {
            TerminalAction::List => {
                // List all active terminals with their current states
                self.registry
                    .list_all_terminals(connection_id)
                    .await
                    .map_err(McpError::Other)?
            }
            TerminalAction::Kill => {
                // Gracefully shutdown terminal and cleanup all resources
                self.registry
                    .kill_terminal(connection_id, args.terminal)
                    .await
                    .map_err(McpError::Other)?
            }
            TerminalAction::Read => {
                // Get current VTE buffer snapshot without executing
                let pwd = ctx.pwd().and_then(|p| p.to_str().map(String::from));
                let terminal = self
                    .registry
                    .find_or_create_terminal(connection_id, args.terminal, pwd)
                    .await
                    .map_err(McpError::Other)?;
                terminal
                    .read_current_state(args.terminal, args.tail)
                    .await
                    .map_err(McpError::Other)?
            }
            TerminalAction::Exec => {
                // Execute command (default action for backward compatibility)
                let command = args.command.ok_or_else(|| {
                    McpError::Other(anyhow::anyhow!(
                        "command field is required for EXEC action"
                    ))
                })?;
                let pwd = ctx.pwd().and_then(|p| p.to_str().map(String::from));
                let terminal = self
                    .registry
                    .find_or_create_terminal(connection_id, args.terminal, pwd)
                    .await
                    .map_err(McpError::Other)?;
                terminal
                    .execute_command(request_id, command, args.clear, args.await_completion_ms, args.tail, Some(ctx))
                    .await
                    .map_err(McpError::Other)?
            }
        };

        // Return typed response with display (terminal output) and metadata (TerminalOutput struct)
        // Terminal output is ONLY in the display field (Vec[Content]0), not duplicated in typed output
        Ok(ToolResponse::new(
            output.output,
            TerminalOutput {
                terminal: output.terminal.or(Some(terminal_id)),
                exit_code: output.exit_code,
                cwd: output.cwd,
                duration_ms: start.elapsed().as_millis() as u64,
                completed: output.completed,
                terminals: output.terminals,
            },
        ))
    }
}

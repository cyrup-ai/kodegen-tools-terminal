//! Terminal tool implementation

use crate::TerminalRegistry;
use kodegen_mcp_schema::terminal::{TERMINAL, TerminalInput};
use kodegen_mcp_tool::error::McpError;
use kodegen_mcp_tool::{Tool, ToolExecutionContext};
use rmcp::model::Content;
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
}

impl Default for TerminalTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for TerminalTool {
    type Args = TerminalInput;
    type PromptArgs = ();

    fn name() -> &'static str {
        TERMINAL
    }

    fn description() -> &'static str {
        "Execute shell commands in persistent, stateful terminal sessions. \
         Terminals maintain environment variables, working directory, and shell \
         state across commands. Use different terminal numbers (1, 2, 3...) for \
         parallel work. Streams output in real-time as the command executes."
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

    fn prompt_arguments() -> Vec<rmcp::model::PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<rmcp::model::PromptMessage>, McpError> {
        Ok(vec![])
    }

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<Vec<Content>, McpError> {
        let connection_id = ctx.connection_id().unwrap_or("default");
        let request_id = ctx.request_id().clone();

        // Get or create terminal from registry
        let terminal = self
            .registry
            .find_or_create_terminal(connection_id, args.terminal)
            .await
            .map_err(McpError::Other)?;

        // Execute command on terminal
        let output = terminal
            .execute_command(request_id, args.command)
            .await
            .map_err(McpError::Other)?;

        Ok(vec![Content::text(serde_json::to_string(&output)?)])
    }
}

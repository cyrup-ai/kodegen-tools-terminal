use crate::manager::TerminalManager;
use kodegen_mcp_schema::terminal::{TERMINAL, TerminalInput, TerminalOutput};
use kodegen_mcp_tool::{Tool, ToolExecutionContext};
use kodegen_mcp_tool::error::McpError;
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageRole, PromptMessageContent};

use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// TOOL IMPLEMENTATION
// ============================================================================

#[derive(Clone)]
pub struct TerminalTool {
    terminal_manager: Arc<TerminalManager>,
}

impl TerminalTool {
    pub fn new(terminal_manager: Arc<TerminalManager>) -> Self {
        Self { terminal_manager }
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

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<Vec<Content>, McpError> {
        let start = Instant::now();
        const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

        // Clone ctx for closures (needed because ctx is moved)
        let ctx_for_cancel = ctx.clone();
        let ctx_for_stream = ctx.clone();

        // Extract connection_id from context (use default for examples/tests)
        let connection_id = ctx.connection_id()
            .unwrap_or("example-session");

        // Delegate to TerminalManager with streaming callback
        let output_response = self.terminal_manager
            .execute_command_with_completion(
                connection_id,
                args.terminal,
                &args.command,
                DEFAULT_TIMEOUT,
                move || ctx_for_cancel.is_cancelled(),
                move |display| {
                    let ctx = ctx_for_stream.clone();
                    async move {
                        ctx.stream(&display).await.ok();
                    }
                },
            )
            .await
            .map_err(McpError::Other)?;

        // Format response
        let final_output = output_response.lines.join("\n");
        let exit_code = output_response.exit_code.unwrap_or(-1);
        let cwd = output_response.cwd
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "/".to_string());

        let output = TerminalOutput {
            terminal: args.terminal,
            output: final_output,
            exit_code,
            cwd,
            duration_ms: start.elapsed().as_millis() as u64,
        };

        let json_str = serde_json::to_string_pretty(&output)
            .map_err(|e| McpError::Other(e.into()))?;
        Ok(vec![Content::text(json_str)])
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I run terminal commands?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The terminal tool executes shell commands in persistent, stateful sessions:\n\n\
                     **Basic usage:**\n\
                     ```json\n\
                     terminal({\"command\": \"ls -la\", \"terminal\": 1})\n\
                     ```\n\n\
                     **Stateful sessions:**\n\
                     Terminals maintain state across commands. Use the same terminal number to reuse:\n\
                     ```json\n\
                     terminal({\"command\": \"cd /tmp\", \"terminal\": 1})\n\
                     terminal({\"command\": \"pwd\", \"terminal\": 1})  // Shows /tmp\n\
                     ```\n\n\
                     **Parallel execution:**\n\
                     Use different terminal numbers for concurrent work:\n\
                     ```json\n\
                     terminal({\"command\": \"npm run build\", \"terminal\": 1})\n\
                     terminal({\"command\": \"cargo test\", \"terminal\": 2})\n\
                     ```\n\n\
                     **Response format:**\n\
                     ```json\n\
                     {\n\
                       \"terminal\": 1,\n\
                       \"output\": \"total 48\\ndrwxr-xr-x  6 user  staff  192 Nov 22 12:34 .\\n...\",\n\
                       \"exit_code\": 0,\n\
                       \"cwd\": \"/Users/user/project\",\n\
                       \"duration_ms\": 123\n\
                     }\n\
                     ```\n\n\
                     **Features:**\n\
                     - Real-time output streaming as command executes\n\
                     - Preserves environment variables and working directory\n\
                     - Supports interactive programs\n\
                     - 5-minute timeout with cancellation support\n\n\
                     **Use cases:**\n\
                     - Build and test commands\n\
                     - Git operations\n\
                     - File system navigation\n\
                     - Package management\n\
                     - Multi-step workflows requiring persistent state",
                ),
            },
        ])
    }
}

use crate::manager::TerminalManager;
use kodegen_mcp_schema::terminal::{StopTerminalCommandArgs, StopTerminalCommandPromptArgs};
use kodegen_mcp_tool::Tool;
use kodegen_mcp_tool::error::McpError;
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;
use std::sync::Arc;

// ============================================================================
// TOOL STRUCT
// ============================================================================

#[derive(Clone)]
pub struct StopTerminalCommandTool {
    terminal_manager: Arc<TerminalManager>,
}

impl StopTerminalCommandTool {
    #[must_use]
    pub fn new(terminal_manager: Arc<TerminalManager>) -> Self {
        Self { terminal_manager }
    }
}

// ============================================================================
// TOOL IMPLEMENTATION
// ============================================================================

impl Tool for StopTerminalCommandTool {
    type Args = StopTerminalCommandArgs;
    type PromptArgs = StopTerminalCommandPromptArgs;

    fn name() -> &'static str {
        kodegen_mcp_schema::terminal::TERMINAL_STOP_COMMAND
    }

    fn description() -> &'static str {
        "Force terminate a running command session by PID. Attempts graceful termination first \
         (SIGTERM), then force kills after 1 second if still running (SIGKILL). Use this to stop \
         long-running commands that you no longer need."
    }

    fn read_only() -> bool {
        false
    }

    fn destructive() -> bool {
        true
    }

    fn open_world() -> bool {
        false
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        // Now force_terminate returns Result, not bool
        self.terminal_manager.force_terminate(args.pid).await?;

        let mut contents = Vec::new();

        // HUMAN VIEW
        let summary = format!(
            "🛑 Process Terminated\nPID: {}\nStatus: Graceful SIGTERM → Force SIGKILL",
            args.pid
        );
        contents.push(Content::text(summary));

        // JSON METADATA
        let metadata = json!({
            "pid": args.pid,
            "success": true,
            "message": format!("Successfully terminated process {}", args.pid)
        });

        let json_str = serde_json::to_string_pretty(&metadata)
            .unwrap_or_else(|_| "{}".to_string());
        contents.push(Content::text(json_str));

        Ok(contents)
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![
            PromptMessage {
                role: PromptMessageRole::User,
                content: PromptMessageContent::text("How do I stop a long-running command?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The stop_terminal_command tool stops running command sessions:\n\n\
                     Basic usage:\n\
                     stop_terminal_command({\"pid\": 12345})\n\n\
                     How it works:\n\
                     1. Takes a PID from start_terminal_command or list_terminal_commands\n\
                     2. Sends graceful termination signal (SIGTERM)\n\
                     3. Waits 1 second for process to exit cleanly\n\
                     4. Force kills if still running (SIGKILL)\n\
                     5. Cleans up PTY and terminal resources\n\
                     6. Returns error if PID doesn't exist\n\n\
                     Typical workflow:\n\
                     1. Start: start_terminal_command({\"command\": \"sleep 3600\"})\n\
                        Returns: {\"pid\": 12345, \"still_running\": true}\n\
                     2. Realize you don't need it anymore\n\
                     3. Stop: stop_terminal_command({\"pid\": 12345})\n\
                        Returns: {\"success\": true}\n\n\
                     When to use:\n\
                     - Command is taking too long\n\
                     - You made a mistake and want to cancel\n\
                     - Process is stuck or hanging\n\
                     - You no longer need the output\n\n\
                     Safety:\n\
                     - Graceful termination first (processes can clean up)\n\
                     - Force kill only after 1 second timeout\n\
                     - Only affects sessions you started via start_terminal_command\n\
                     - Returns error if PID doesn't exist or already terminated\n\n\
                     Example with list_terminal_commands:\n\
                     1. list_terminal_commands() → [{\"pid\": 12345, \"still_running\": true}]\n\
                     2. stop_terminal_command({\"pid\": 12345}) → Stop that specific process\n\n\
                     Note: After termination, the process will move to completed sessions \
                     and you can still retrieve its partial output via read_terminal_output.",
                ),
            },
        ])
    }
}

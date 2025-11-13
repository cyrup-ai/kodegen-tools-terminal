use crate::manager::{CommandManager, TerminalManager};
use kodegen_mcp_schema::terminal::{StartTerminalCommandArgs, StartTerminalCommandPromptArgs};
use kodegen_mcp_tool::Tool;
use kodegen_mcp_tool::error::McpError;
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageContent, PromptMessageRole};
use serde_json::json;
use std::sync::Arc;

// ============================================================================
// TOOL STRUCT
// ============================================================================

#[derive(Clone)]
pub struct StartTerminalCommandTool {
    terminal_manager: Arc<TerminalManager>,
    command_manager: CommandManager,
}

impl StartTerminalCommandTool {
    #[must_use]
    pub fn new(terminal_manager: Arc<TerminalManager>, command_manager: CommandManager) -> Self {
        Self {
            terminal_manager,
            command_manager,
        }
    }
}

// ============================================================================
// TOOL IMPLEMENTATION
// ============================================================================

impl Tool for StartTerminalCommandTool {
    type Args = StartTerminalCommandArgs;
    type PromptArgs = StartTerminalCommandPromptArgs;

    fn name() -> &'static str {
        "terminal_start_command"
    }

    fn description() -> &'static str {
        "Execute a shell command with full terminal emulation. Supports long-running commands, \
         output streaming, and session management. Returns PID for tracking and initial output. \
         Use read_terminal_output to get more output from long-running commands."
    }

    fn read_only() -> bool {
        false
    }

    fn destructive() -> bool {
        true
    }

    fn open_world() -> bool {
        true
    }

    async fn execute(&self, args: Self::Args) -> Result<Vec<Content>, McpError> {
        // Validate command against blocked list
        let is_allowed = self.command_manager.validate_command(&args.command);
        if !is_allowed {
            return Err(McpError::PermissionDenied(format!(
                "Command not allowed: {}. This command is in the blocked list for safety.",
                args.command
            )));
        }

        // Execute via terminal manager
        let result = self
            .terminal_manager
            .execute_command(
                &args.command,
                Some(args.initial_delay_ms),
                args.shell.as_deref(),
            )
            .await
            .map_err(McpError::Other)?;

        let mut contents = Vec::new();

        // HUMAN VIEW - LAST 3 LINES OF INITIAL OUTPUT
        let output_lines: Vec<&str> = result.output.lines().collect();
        let last_3: Vec<&str> = output_lines
            .iter()
            .rev()
            .take(3)
            .rev()
            .copied()
            .collect();

        let summary = if result.ready_for_input {
            format!(
                "🚀 REPL Started • PID {}\nReady for input\nLast output:\n{}",
                result.pid,
                last_3.join("\n")
            )
        } else if result.is_blocked {
            format!(
                "⏳ Command Running • PID {}\nUse terminal_read_output to get more\nLast output:\n{}",
                result.pid,
                last_3.join("\n")
            )
        } else {
            format!(
                "✓ Command Complete • PID {}\nOutput:\n{}",
                result.pid,
                last_3.join("\n")
            )
        };
        contents.push(Content::text(summary));

        // JSON METADATA (full output preserved)
        let metadata = json!({
            "pid": result.pid,
            "output": result.output,
            "is_blocked": result.is_blocked,
            "ready_for_input": result.ready_for_input,
            "message": if result.ready_for_input {
                format!(
                    "REPL ready for input (PID: {}). Use terminal_send_input({{\"pid\": {}, \"input\": \"...\"}}) to interact.",
                    result.pid, result.pid
                )
            } else if result.is_blocked {
                format!(
                    "Command still running (PID: {}). Use terminal_read_output({{\"pid\": {}}}) to get more output.",
                    result.pid, result.pid
                )
            } else {
                "Command completed.".to_string()
            }
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
                content: PromptMessageContent::text("How do I execute shell commands?"),
            },
            PromptMessage {
                role: PromptMessageRole::Assistant,
                content: PromptMessageContent::text(
                    "The start_terminal_command tool runs shell commands with full terminal support:\n\n\
                     Basic usage:\n\
                     start_terminal_command({\"command\": \"ls -la\"})\n\n\
                     With custom initial delay (waits longer before returning):\n\
                     start_terminal_command({\"command\": \"npm install\", \"initial_delay_ms\": 1000})\n\n\
                     With specific shell:\n\
                     start_terminal_command({\"command\": \"echo $SHELL\", \"shell\": \"/bin/bash\"})\n\n\
                     Key features:\n\
                     - Full PTY support for interactive commands\n\
                     - Session tracking with PID for long-running commands\n\
                     - Output streaming (use read_terminal_output for more output)\n\
                     - Default initial_delay_ms is 100ms (brief wait for quick commands)\n\
                     - Command validation for safety (blocks dangerous commands)\n\n\
                     For long-running commands:\n\
                     1. start_terminal_command returns PID after initial_delay_ms\n\
                     2. Command continues running in background\n\
                     3. Use read_terminal_output({\"pid\": <pid>}) to get ongoing output\n\
                     4. Use stop_terminal_command({\"pid\": <pid>}) to stop if needed\n\
                     5. Use list_terminal_commands() to see all active sessions\n\n\
                     Security:\n\
                     - Blocked commands: rm, sudo, chmod, kill, wget, curl, etc.\n\
                     - Complex command parsing handles pipes, redirects, subshells\n\
                     - Safe error handling throughout",
                ),
            },
        ])
    }
}

use crate::manager::TerminalManager;
use kodegen_mcp_schema::terminal::{TERMINAL, TerminalInput, TerminalOutput};
use kodegen_mcp_tool::{Tool, ToolExecutionContext};
use kodegen_mcp_tool::error::McpError;
use rmcp::model::{Content, PromptArgument, PromptMessage, PromptMessageRole, PromptMessageContent};

use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;

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

        // ========== PHASE 1: INITIALIZATION ==========

        // Extract connection_id from context (use default for examples/tests)
        let connection_id = ctx.connection_id()
            .unwrap_or("example-session");

        let terminal_id = args.terminal;

        // Terminal reuse logic: Check if terminal exists
        let terminal_exists = self.terminal_manager
            .get_session(connection_id, terminal_id)
            .await
            .is_some();

        if !terminal_exists {
            // Create new interactive shell (no command sent yet)
            self.terminal_manager
                .spawn_command(connection_id, terminal_id, None)
                .await
                .map_err(McpError::Other)?;
        }

        // ========== PHASE 2: STREAMING SETUP ==========

        // Subscribe to broadcast channel BEFORE sending command (avoid race condition)
        let mut output_rx = self.terminal_manager
            .subscribe_output(connection_id, terminal_id)
            .await
            .ok_or_else(|| McpError::Other(anyhow::anyhow!(
                "Terminal session not found after creation"
            )))?;

        // Send command to shell (works for both new and existing terminals)
        self.terminal_manager
            .send_input(connection_id, terminal_id, &args.command, true)
            .await
            .map_err(McpError::Other)?;

        // ========== PHASE 3: REAL-TIME STREAMING LOOP ==========

        let mut last_output = String::new();

        loop {
            // Check cancellation
            if ctx.is_cancelled() {
                self.terminal_manager
                    .force_terminate(connection_id, terminal_id)
                    .await
                    .ok();
                return Err(McpError::Other(
                    anyhow::anyhow!("Command execution cancelled by user")
                ));
            }

            // Check timeout
            if start.elapsed() > DEFAULT_TIMEOUT {
                // Get final output before terminating
                let final_output = self.terminal_manager
                    .get_output(connection_id, terminal_id, 0, usize::MAX)
                    .await
                    .map(|r| r.lines.join("\n"))
                    .unwrap_or_else(|| last_output.clone());

                self.terminal_manager
                    .force_terminate(connection_id, terminal_id)
                    .await
                    .ok();

                return Err(McpError::Other(anyhow::anyhow!(
                    "Command timed out after 5 minutes. Last output:\n{}", final_output
                )));
            }

            // Try to receive screen update notification (non-blocking with timeout)
            match tokio::time::timeout(Duration::from_millis(100), output_rx.recv()).await {
                Ok(Ok(())) => {
                    // Screen updated - get actual content
                    let screen_content = self.terminal_manager
                        .get_output(connection_id, terminal_id, 0, usize::MAX)
                        .await
                        .map(|r| r.lines.join("\n"))
                        .unwrap_or_default();

                    last_output = screen_content.clone();

                    // Truncate for streaming (last 30 lines or 2000 chars)
                    let display = truncate_for_streaming(&screen_content);

                    // Stream to user (fire-and-forget)
                    ctx.stream(&display).await.ok();
                }
                Ok(Err(RecvError::Lagged(n))) => {
                    // Missed some messages due to lag - resubscribe and continue
                    log::warn!("Output stream lagged by {} messages (resubscribing)", n);

                    output_rx = self.terminal_manager
                        .subscribe_output(connection_id, terminal_id)
                        .await
                        .ok_or_else(|| McpError::Other(anyhow::anyhow!("Terminal closed")))?;
                }
                Ok(Err(RecvError::Closed)) => {
                    // Channel closed - terminal finished
                    log::info!("Broadcast channel closed, command completed");
                    break;
                }
                Err(_) => {
                    // Timeout - check if terminal is still running via authoritative get_output()
                    let output_response = self.terminal_manager
                        .get_output(connection_id, terminal_id, 0, 1)
                        .await;

                    if let Some(resp) = output_response {
                        if resp.is_complete {
                            log::info!("Terminal completed (detected via get_output)");
                            break;
                        }
                    } else {
                        // Session not found - terminal was cleaned up
                        log::warn!("Terminal session not found (possibly cleaned up)");
                        break;
                    }
                }
            }
        }

        // ========== PHASE 4: FINAL OUTPUT COLLECTION ==========

        // Get complete, authoritative output from Alacritty Grid
        let output_response = self.terminal_manager
            .get_output(connection_id, terminal_id, 0, usize::MAX)
            .await
            .ok_or_else(|| McpError::Other(anyhow::anyhow!(
                "Terminal not found after completion"
            )))?;

        let final_output = output_response.lines.join("\n");
        let exit_code = output_response.exit_code.unwrap_or(-1);

        // Get CWD
        let cwd = self.terminal_manager
            .get_terminal_cwd(connection_id, terminal_id)
            .await
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "/".to_string());

        let output = TerminalOutput {
            terminal: terminal_id,
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

// ============================================================================
// HELPER FUNCTIONS
// ============================================================================

/// Truncate output for streaming to avoid overwhelming the client
///
/// Shows last 30 lines or 2000 chars, whichever is smaller.
/// Final output via get_output() is never truncated.
fn truncate_for_streaming(content: &str) -> String {
    const MAX_STREAM_CHARS: usize = 2000;
    const MAX_STREAM_LINES: usize = 30;

    if content.len() <= MAX_STREAM_CHARS {
        return content.to_string();
    }

    let lines: Vec<&str> = content.lines().collect();

    if lines.len() <= MAX_STREAM_LINES {
        return content.to_string();
    }

    // Show last N lines
    let tail_lines = &lines[lines.len().saturating_sub(MAX_STREAM_LINES)..];
    format!(
        "...\n[{} earlier lines omitted for streaming]\n{}",
        lines.len() - tail_lines.len(),
        tail_lines.join("\n")
    )
}

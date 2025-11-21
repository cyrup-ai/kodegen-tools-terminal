use crate::manager::TerminalManager;
use kodegen_mcp_schema::terminal::{TerminalRunCommandArgs, TerminalRunCommandPromptArgs, TERMINAL_RUN_COMMAND};
use kodegen_mcp_tool::{Tool, ToolExecutionContext};
use kodegen_mcp_tool::error::McpError;
use rmcp::model::{Content, PromptArgument, PromptMessage};

use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};

// ============================================================================
// OUTPUT SCHEMA
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct TerminalRunCommandOutput {
    /// Process ID
    pub pid: u32,

    /// Process exit code (0 = success)
    pub exit_code: Option<i32>,

    /// Complete command output (stdout + stderr interleaved)
    pub output: String,

    /// Number of output lines
    pub output_lines: usize,

    /// Execution duration in seconds
    pub duration_seconds: f64,

    /// Whether command timed out
    pub timed_out: bool,
}

// ============================================================================
// TOOL IMPLEMENTATION
// ============================================================================

#[derive(Clone)]
pub struct TerminalRunCommandTool {
    terminal_manager: Arc<TerminalManager>,
}

impl TerminalRunCommandTool {
    pub fn new(terminal_manager: Arc<TerminalManager>) -> Self {
        Self { terminal_manager }
    }
}

impl Tool for TerminalRunCommandTool {
    type Args = TerminalRunCommandArgs;
    type PromptArgs = TerminalRunCommandPromptArgs;

    fn name() -> &'static str {
        TERMINAL_RUN_COMMAND
    }

    fn description() -> &'static str {
        "Execute a shell command with full terminal emulation. \
         Blocks until completion, streaming last line of output via progress. \
         Returns complete output and exit code when finished."
    }

    fn read_only() -> bool {
        false // Commands can modify filesystem
    }

    fn destructive() -> bool {
        true // Commands can delete files
    }

    fn idempotent() -> bool {
        false // Same command may have different effects
    }

    fn open_world() -> bool {
        true // Commands may access network
    }

    async fn execute(
        &self,
        args: Self::Args,
        ctx: ToolExecutionContext,
    ) -> Result<Vec<Content>, McpError> {
        let start = Instant::now();
        const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes
        
        // Spawn command - convert anyhow::Error to McpError
        let pid = self.terminal_manager
            .spawn_command(&args.command, args.shell.as_deref())
            .await
            .map_err(McpError::Other)?;

        // Wait for initial delay to let quick commands complete
        let initial_delay = Duration::from_millis(args.initial_delay_ms);
        tokio::time::sleep(initial_delay).await;

        let mut last_notified_line = String::new();
        let mut last_notification = Instant::now();
        const NOTIFICATION_INTERVAL: Duration = Duration::from_millis(500);

        // Poll loop
        loop {
            // Check cancellation
            if ctx.is_cancelled() {
                self.terminal_manager.force_terminate(pid).await.ok();
                return Err(McpError::Other(
                    anyhow::anyhow!("Command execution cancelled by user")
                ));
            }

            // Check timeout
            if start.elapsed() > DEFAULT_TIMEOUT {
                self.terminal_manager.force_terminate(pid).await.ok();
                
                let partial = self.terminal_manager
                    .get_output(pid, 0, 100_000)
                    .await;

                let (output_text, output_lines) = if let Some(resp) = partial {
                    (resp.lines.join("\n"), resp.total_lines)
                } else {
                    (String::new(), 0)
                };

                let output = TerminalRunCommandOutput {
                    pid,
                    exit_code: None,
                    output: output_text,
                    output_lines,
                    duration_seconds: start.elapsed().as_secs_f64(),
                    timed_out: true,
                };

                let json_str = serde_json::to_string_pretty(&output)
                    .map_err(|e| McpError::Other(e.into()))?;
                return Ok(vec![Content::text(json_str)])
            }

            // Get current output
            let current = self.terminal_manager
                .get_output(pid, 0, 100_000)
                .await;

            if let Some(resp) = current {
                // Extract and stream last non-empty line
                if let Some(line) = resp.lines.iter().rev().find(|l| !l.trim().is_empty())
                    && line != &last_notified_line 
                    && last_notification.elapsed() >= NOTIFICATION_INTERVAL 
                {
                    let display = if line.len() > 500 {
                        format!("{}... [truncated]", &line[..500])
                    } else {
                        line.clone()
                    };
                    
                    ctx.stream(&display).await.ok();
                    
                    last_notified_line = line.clone();
                    last_notification = Instant::now();
                }

                // Check completion
                if resp.is_complete {
                    let output = TerminalRunCommandOutput {
                        pid,
                        exit_code: resp.exit_code,
                        output: resp.lines.join("\n"),
                        output_lines: resp.total_lines,
                        duration_seconds: start.elapsed().as_secs_f64(),
                        timed_out: false,
                    };

                    let json_str = serde_json::to_string_pretty(&output)
                        .map_err(|e| McpError::Other(e.into()))?;
                    return Ok(vec![Content::text(json_str)])
                }
            }

            // Sleep before next poll
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    fn prompt_arguments() -> Vec<PromptArgument> {
        vec![]
    }

    async fn prompt(&self, _args: Self::PromptArgs) -> Result<Vec<PromptMessage>, McpError> {
        Ok(vec![])
    }
}

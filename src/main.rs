//! Terminal Category HTTP Server
//!
//! Serves terminal tools via HTTP/HTTPS transport using kodegen_server_http.

use anyhow::Result;
use kodegen_server_http::{Managers, RouterSet, register_tool, run_http_server};
use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    run_http_server("terminal", |_config, _tracker| {
        Box::pin(async move {
            let tool_router = ToolRouter::new();
            let prompt_router = PromptRouter::new();
            let managers = Managers::new();

            // Create managers for terminal tools
            let terminal_manager = Arc::new(kodegen_tools_terminal::TerminalManager::new());

            // Register 2 terminal tools (was 5)
            let (tool_router, prompt_router) = register_tool(
                tool_router,
                prompt_router,
                kodegen_tools_terminal::TerminalRunCommandTool::new(terminal_manager.clone()),
            );

            let (tool_router, prompt_router) = register_tool(
                tool_router,
                prompt_router,
                kodegen_tools_terminal::SendTerminalInputTool::new(terminal_manager.clone()),
            );

            // CRITICAL: Start cleanup task after all tools are registered
            terminal_manager.start_cleanup_task();

            Ok(RouterSet::new(tool_router, prompt_router, managers))
        })
    })
    .await
}

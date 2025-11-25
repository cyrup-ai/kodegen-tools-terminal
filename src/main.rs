//! Terminal Category HTTP Server
//!
//! Serves terminal tools via HTTP/HTTPS transport using kodegen_server_http.

use anyhow::Result;
use kodegen_server_http::{Managers, RouterSet, register_tool, run_http_server};
use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};

#[tokio::main]
async fn main() -> Result<()> {
    run_http_server("terminal", |_config, _tracker| {
        Box::pin(async move {
            let tool_router = ToolRouter::new();
            let prompt_router = PromptRouter::new();
            let managers = Managers::new();

            // Register terminal tool
            let (tool_router, prompt_router) = register_tool(
                tool_router,
                prompt_router,
                kodegen_tools_terminal::TerminalTool::new(),
            );

            Ok(RouterSet::new(tool_router, prompt_router, managers))
        })
    })
    .await
}

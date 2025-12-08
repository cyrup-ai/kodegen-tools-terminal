//! Terminal Category HTTP Server
//!
//! Serves terminal tools via HTTP/HTTPS transport using kodegen_server_http.

use anyhow::Result;
use kodegen_config::CATEGORY_TERMINAL;
use kodegen_server_http::{ConnectionCleanupFn, Managers, RouterSet, ServerBuilder, register_tool};
use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<()> {
    ServerBuilder::new()
        .category(CATEGORY_TERMINAL)
        .register_tools(|| async {
            let tool_router = ToolRouter::new();
            let prompt_router = PromptRouter::new();
            let managers = Managers::new();

            // Create terminal tool and get registry for cleanup
            let terminal_tool = kodegen_tools_terminal::TerminalTool::new();
            let terminal_registry = terminal_tool.registry();

            // Register terminal tool
            let (tool_router, prompt_router) = register_tool(
                tool_router,
                prompt_router,
                terminal_tool,
            );

            // Create async cleanup handler
            let cleanup: ConnectionCleanupFn = Arc::new(move |connection_id: String| {
                let registry = terminal_registry.clone();
                Box::pin(async move {
                    let cleaned = registry.cleanup_connection(&connection_id).await;
                    log::info!(
                        "Connection {}: cleaned up {} terminal session(s)",
                        connection_id,
                        cleaned
                    );
                }) as Pin<Box<dyn Future<Output = ()> + Send + 'static>>
            });

            let mut router_set = RouterSet::new(tool_router, prompt_router, managers);
            router_set.connection_cleanup = Some(cleanup);
            Ok(router_set)
        })
        .run()
        .await
}

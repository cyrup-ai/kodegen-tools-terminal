pub mod manager;
pub mod pty;

pub mod terminal;

pub use terminal::TerminalTool;
pub use manager::{
    ActiveTerminalSession, CommandManager, CompletedTerminalSession, TerminalCommandResult,
    TerminalManager, TerminalOutputResponse,
};

/// Start the HTTP server programmatically for embedded mode
///
/// This is called by kodegend instead of spawning an external process.
/// Blocks until the server shuts down.
///
/// # Arguments
/// * `addr` - Socket address to bind to (e.g., "127.0.0.1:30438")
/// * `tls_cert` - Optional path to TLS certificate file
/// * `tls_key` - Optional path to TLS private key file
///
/// # Returns
/// ServerHandle for graceful shutdown, or error if startup fails
pub async fn start_server(
    addr: std::net::SocketAddr,
    tls_cert: Option<std::path::PathBuf>,
    tls_key: Option<std::path::PathBuf>,
) -> anyhow::Result<kodegen_server_http::ServerHandle> {
    use kodegen_server_http::{Managers, RouterSet, create_http_server, register_tool};
    use rmcp::handler::server::router::{prompt::PromptRouter, tool::ToolRouter};
    use std::sync::Arc;
    use std::time::Duration;

    let tls_config = match (tls_cert, tls_key) {
        (Some(cert), Some(key)) => Some((cert, key)),
        _ => None,
    };

    let shutdown_timeout = Duration::from_secs(30);
    let session_keep_alive = Duration::ZERO;

    create_http_server(
        "terminal",
        addr,
        tls_config,
        shutdown_timeout,
        session_keep_alive,
        |config: &kodegen_config_manager::ConfigManager, _tracker| {
            let _config = config.clone();
            Box::pin(async move {
                let tool_router = ToolRouter::new();
                let prompt_router = PromptRouter::new();
                let managers = Managers::new();

                // Create managers for terminal tools
                let terminal_manager = Arc::new(crate::TerminalManager::new());

                // Register unified terminal tool
                let (tool_router, prompt_router) = register_tool(
                    tool_router,
                    prompt_router,
                    crate::TerminalTool::new(terminal_manager.clone()),
                );

                // Start cleanup task
                terminal_manager.start_cleanup_task();

                Ok(RouterSet::new(tool_router, prompt_router, managers))
            })
        },
    )
    .await
}

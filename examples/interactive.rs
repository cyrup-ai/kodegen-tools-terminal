//! Example demonstrating interactive prompt handling
//!
//! This tests that the terminal can handle commands that prompt for user input.
//! These commands will likely HANG because:
//! - The command waits for input on stdin
//! - There's no way to send input to the running command
//!
//! Expected behavior:
//! - Command should hang waiting for input
//! - await_completion_ms timeout should trigger and return control
//! - READ action should still work (not block)
//! - KILL action should terminate the stuck process
//!
//! Run with: cargo run --example interactive

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL TOOL: INTERACTIVE PROMPT TEST ===\n");

    // Create registry
    let registry = TerminalRegistry::new();

    // Test: Command that prompts for input (should hang or timeout)
    test_interactive_prompt(&registry).await?;

    log::info!("\n=== INTERACTIVE PROMPT TEST COMPLETED ===");
    log::info!("\n💀 CLEANUP: Killing terminal\n");

    // Kill terminal 0
    log::info!("💀 Killing terminal 0...");
    let output0 = registry.kill_terminal("demo-connection", 0).await?;
    log::info!("   ✅ Terminal 0 killed: exit_code={:?}, duration={}ms", output0.exit_code, output0.duration_ms);

    log::info!("\n✅ Test complete, exiting main()");

    Ok(())
}

/// Test: Command that waits for stdin input
///
/// Uses `head -1` which reads one line from stdin and waits forever if none provided.
/// This tests whether:
/// 1. The timeout (await_completion_ms) works correctly
/// 2. The terminal doesn't enter an unrecoverable state
async fn test_interactive_prompt(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    // `head -1` waits for one line of input from stdin
    // This will hang forever since no input is provided
    let command = "echo 'Waiting for input...' && head -1";

    log::info!("🧪 TEST: Interactive Prompt (head -1 waiting for stdin)");
    log::info!("   Command: {}", command);
    log::info!("   Timeout: 5 seconds");
    log::info!("   Expected: Should timeout after 5 seconds, NOT hang forever");

    let terminal = registry.find_or_create_terminal("demo-connection", 0).await?;
    let request_id = rmcp::model::RequestId::String("test-interactive".to_string().into());

    let output = terminal.execute_command(
        request_id,
        command.to_string(),
        true,  // clear
        5_000, // 5 second timeout
        2000,
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   Result:");
    log::info!("      Terminal: {:?}", output.terminal);
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      CWD: {}", output.cwd);
    log::info!("      Duration: {}ms", output.duration_ms);
    log::info!("      Completed: {}", output.completed);
    log::info!("      Output:\n{}", output.output);

    Ok(())
}

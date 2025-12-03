//! Example demonstrating complex command handling (for loops)
//!
//! This tests that the terminal can handle complex shell constructs like:
//! - For loops iterating over directories
//! - Multi-command pipelines
//!
//! Run with: cargo run --example complex_command

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL TOOL: COMPLEX COMMAND TEST ===\n");

    // Create registry
    let registry = TerminalRegistry::new();

    // Test: For loop listing packages
    test_for_loop_packages(&registry).await?;

    log::info!("\n=== COMPLEX COMMAND TEST COMPLETED ===");
    log::info!("\n💀 CLEANUP: Killing terminal\n");

    // Kill terminal 0
    log::info!("💀 Killing terminal 0...");
    let output0 = registry.kill_terminal("demo-connection", 0).await?;
    log::info!("   ✅ Terminal 0 killed: exit_code={:?}, duration={}ms", output0.exit_code, output0.duration_ms);

    log::info!("\n✅ Test complete, exiting main()");
    
    Ok(())
}

/// Test: For loop with actual newline characters (like JSON deserializes \n)
async fn test_for_loop_packages(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    // Command with ACTUAL newline characters (char code 10), not escaped \n
    let command = "cd /Volumes/samsung_t9/kodegen-workspace && for pkg in packages/*\ndo\n  echo \"=== $pkg ===\"\ndone";
    
    log::info!("🧪 TEST: For Loop with Embedded Newlines");
    log::info!("   Command (showing \\n as newlines):");
    log::info!("{}", command);

    let terminal = registry.find_or_create_terminal("demo-connection", 0).await?;
    let request_id = rmcp::model::RequestId::String("test-for-loop".to_string().into());

    let output = terminal.execute_command(
        request_id,
        command.to_string(),
        true,   // clear
        60_000, // 60 second timeout
        2000,
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   ✅ Command completed:");
    log::info!("      Terminal: {:?}", output.terminal);
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      CWD: {}", output.cwd);
    log::info!("      Duration: {}ms", output.duration_ms);
    log::info!("      Completed: {}", output.completed);
    log::info!("      Output:\n{}", output.output);

    Ok(())
}

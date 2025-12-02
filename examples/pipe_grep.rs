//! Example testing that grep works normally (no longer intercepted by builtin)
//!
//! This verifies that:
//! 1. `echo "test" | grep test` - pipes work normally
//! 2. `grep pattern file.txt` - direct grep also works (uses system grep)
//! 3. Complex pipes with grep work
//!
//! Run with: cargo run --example pipe_grep

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL TOOL: GREP PIPE TEST ===\n");

    // Create registry
    let registry = TerminalRegistry::new();

    // Test 1: Grep in a pipe
    test_pipe_grep(&registry).await?;

    // Test 2: Complex pipes with grep
    test_complex_pipe_grep(&registry).await?;

    log::info!("\n=== GREP PIPE TEST COMPLETED ===");
    log::info!("\n💀 CLEANUP: Killing terminal\n");

    // Kill terminal 0
    log::info!("💀 Killing terminal 0...");
    let output0 = registry.kill_terminal("demo-connection", 0).await?;
    log::info!("   ✅ Terminal 0 killed: exit_code={:?}, duration={}ms", output0.exit_code, output0.duration_ms);

    log::info!("\n✅ Test complete, exiting main()");

    Ok(())
}

/// Test 1: Grep in a pipe should work normally
async fn test_pipe_grep(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    let command = r#"echo "hello world test" | grep test"#;

    log::info!("🧪 TEST 1: Grep in Pipe");
    log::info!("   Command: {}", command);

    let terminal = registry.find_or_create_terminal("demo-connection", 0).await?;
    let request_id = rmcp::model::RequestId::String("test-pipe-grep".to_string().into());

    let output = terminal.execute_command(
        request_id,
        command.to_string(),
        30_000, // 30 second timeout
        2000,
    ).await?;

    log::info!("   Result:");
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      Output: {}", output.output.trim());

    // Verify: Should succeed (exit 0) and output should contain "test"
    let success = output.exit_code == Some(0) && output.output.contains("hello world test");
    if success {
        log::info!("   ✅ PASS: Pipe grep worked normally");
    } else {
        log::error!("   ❌ FAIL: Pipe grep failed");
        log::error!("      Expected: exit_code=0, output containing 'hello world test'");
        log::error!("      Got: exit_code={:?}, output={}", output.exit_code, output.output);
    }

    Ok(())
}

/// Test 2: Complex pipe with grep
async fn test_complex_pipe_grep(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    let command = r#"ls /tmp | head -5 | grep -v "^$""#;

    log::info!("\n🧪 TEST 2: Complex Pipe with Grep");
    log::info!("   Command: {}", command);

    let terminal = registry.find_or_create_terminal("demo-connection", 0).await?;
    let request_id = rmcp::model::RequestId::String("test-complex-pipe".to_string().into());

    let output = terminal.execute_command(
        request_id,
        command.to_string(),
        30_000, // 30 second timeout
        2000,
    ).await?;

    log::info!("   Result:");
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      Output:\n{}", output.output.trim());

    // Verify: exit code 0 means matches found, 1 means no matches (both are valid)
    let success = output.exit_code == Some(0) || output.exit_code == Some(1);
    if success {
        log::info!("   ✅ PASS: Complex pipe grep worked normally");
    } else {
        log::error!("   ❌ FAIL: Complex pipe grep failed unexpectedly");
        log::error!("      Got: exit_code={:?}", output.exit_code);
    }

    Ok(())
}

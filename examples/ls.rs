use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL EXAMPLE ===\n");

    // Create registry and get terminal
    let registry = TerminalRegistry::new();
    let terminal1 = registry.find_or_create_terminal("test-connection", 1).await?;

    // Run test scenarios
    test_basic_command(&terminal1).await?;
    log::info!("\n{}\n", "=".repeat(80));

    test_terminal_reuse(&terminal1).await?;

    log::info!("\n=== ALL SCENARIOS COMPLETED ===");
    Ok(())
}

/// Test basic command execution
async fn test_basic_command(terminal: &std::sync::Arc<kodegen_tools_terminal::Terminal>) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("TEST: Basic command execution");

    let request_id = rmcp::model::RequestId::String("test-basic-1".to_string().into());
    let output = terminal.execute_command(request_id, "ls -la".to_string()).await?;

    log::info!("  Terminal: {}", output.terminal);
    log::info!("  Exit code: {}", output.exit_code);
    log::info!("  CWD: {}", output.cwd);
    log::info!("  Duration: {}ms", output.duration_ms);
    log::info!("  Output length: {} bytes", output.output.len());

    assert_eq!(output.exit_code, 0, "ls command should succeed");
    assert!(!output.output.is_empty(), "ls should produce output");

    Ok(())
}

/// Test terminal reuse (state persistence)
async fn test_terminal_reuse(terminal: &std::sync::Arc<kodegen_tools_terminal::Terminal>) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("TEST: Terminal reuse (state persistence)");

    log::info!("\n  Step 1: Create /tmp/kodegen_test and cd into it");
    let request_id1 = rmcp::model::RequestId::String("test-reuse-1".to_string().into());
    let output1 = terminal.execute_command(
        request_id1,
        "mkdir -p /tmp/kodegen_test && cd /tmp/kodegen_test && pwd".to_string()
    ).await?;
    log::info!("    CWD: {}", output1.cwd);
    assert!(output1.cwd.contains("kodegen_test"), "Should be in kodegen_test directory");

    log::info!("\n  Step 2: Create file (same terminal - state should persist)");
    let request_id2 = rmcp::model::RequestId::String("test-reuse-2".to_string().into());
    let output2 = terminal.execute_command(
        request_id2,
        "echo 'Hello Terminal 1' > test.txt && pwd".to_string()
    ).await?;
    log::info!("    CWD: {}", output2.cwd);
    assert_eq!(output2.exit_code, 0, "Command should succeed");
    assert!(output2.cwd.contains("kodegen_test"), "Should still be in kodegen_test");

    log::info!("\n  Step 3: Verify state persisted");
    let request_id3 = rmcp::model::RequestId::String("test-reuse-3".to_string().into());
    let output3 = terminal.execute_command(
        request_id3,
        "pwd && cat test.txt".to_string()
    ).await?;
    log::info!("    Output: {}", output3.output.trim());
    assert!(output3.output.contains("kodegen_test"), "Should still be in kodegen_test directory");
    assert!(output3.output.contains("Hello Terminal 1"), "File should contain expected content");

    log::info!("  ✅ Terminal state persisted!");

    // Cleanup
    let request_id_cleanup = rmcp::model::RequestId::String("test-reuse-cleanup".to_string().into());
    let _ = terminal.execute_command(request_id_cleanup, "cd /tmp && rm -rf /tmp/kodegen_test".to_string()).await;

    Ok(())
}

use kodegen_tools_terminal::TerminalManager;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::broadcast::error::RecvError;

const CONNECTION_ID: &str = "example-session";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL MANAGER EXAMPLE ===\n");

    // Create terminal manager
    let terminal_manager = Arc::new(TerminalManager::new());

    // Run all scenarios
    test_terminal_reuse(&terminal_manager).await?;
    log::info!("\n{}\n", "=".repeat(80));

    test_parallel_terminals(&terminal_manager).await?;
    log::info!("\n{}\n", "=".repeat(80));

    test_error_handling(&terminal_manager).await?;

    log::info!("\n=== ALL SCENARIOS COMPLETED ===");
    Ok(())
}

/// Helper: spawn shell, send command, wait for completion using subscribe_output
async fn spawn_and_wait(
    manager: &TerminalManager,
    connection_id: &str,
    terminal_id: u32,
    command: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. Spawn interactive shell (no command sent yet)
    manager.spawn_command(connection_id, terminal_id, None).await?;

    // 2. Subscribe to output BEFORE sending command (avoid race condition)
    let mut output_rx = manager.subscribe_output(connection_id, terminal_id).await
        .ok_or("Terminal not found after spawn")?;

    // 3. Send command to shell via stdin
    manager.send_input(connection_id, terminal_id, command, true).await?;

    // 4. Event-driven completion detection with periodic polling
    loop {
        // Use timeout to periodically check is_complete even if no output broadcast
        match tokio::time::timeout(
            std::time::Duration::from_millis(50),
            output_rx.recv()
        ).await {
            Ok(Ok(_screen_content)) => {
                // Received output update - check if command completed
                if let Some(resp) = manager.get_output(connection_id, terminal_id, 0, 1).await
                    && resp.is_complete
                {
                    break;  // Command finished!
                }
            }
            Ok(Err(RecvError::Lagged(_))) => {
                // Missed messages - resubscribe and continue
                output_rx = manager.subscribe_output(connection_id, terminal_id).await
                    .ok_or("Terminal closed")?;
            }
            Ok(Err(RecvError::Closed)) => {
                // Graceful fallback (won't occur with Arc<broadcast::Sender> in struct)
                break;
            }
            Err(_timeout) => {
                // Timeout - check is_complete flag periodically
                if let Some(resp) = manager.get_output(connection_id, terminal_id, 0, 1).await
                    && resp.is_complete
                {
                    break;  // Command finished!
                }
            }
        }
    }

    Ok(())
}

async fn test_terminal_reuse(
    manager: &Arc<TerminalManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("SCENARIO 1: Terminal Reuse (State Persistence)");
    log::info!("{}", "=".repeat(80));

    log::info!("\nStep 1: Create /tmp/kodegen_test and cd into it");
    spawn_and_wait(
        manager,
        CONNECTION_ID,
        1,
        "mkdir -p /tmp/kodegen_test && cd /tmp/kodegen_test && pwd",
    ).await?;

    let output1 = manager.get_output(CONNECTION_ID, 1, 0, usize::MAX).await
        .ok_or("Terminal 1 not found")?;
    log::info!("  Output: {}", output1.lines.join("\n").trim());
    assert!(output1.is_complete, "Command should be complete");

    log::info!("\nStep 2: Create file (same terminal - reuse)");
    manager.send_input(CONNECTION_ID, 1, "echo 'Hello Terminal 1' > test.txt\n", false).await?;

    let output2 = manager.get_output(CONNECTION_ID, 1, 0, usize::MAX).await
        .ok_or("Terminal 1 not found")?;
    assert_eq!(output2.exit_code, Some(0), "Command should succeed");

    log::info!("\nStep 3: Verify state persisted");
    manager.send_input(CONNECTION_ID, 1, "pwd && cat test.txt\n", false).await?;

    let output3 = manager.get_output(CONNECTION_ID, 1, 0, usize::MAX).await
        .ok_or("Terminal 1 not found")?;

    let output_text = output3.lines.join("\n");
    log::info!("  Output:\n{}", output_text);
    assert!(output_text.contains("kodegen_test"), "Should still be in kodegen_test directory");
    assert!(output_text.contains("Hello Terminal 1"), "File should contain expected content");

    let cwd = manager.get_terminal_cwd(CONNECTION_ID, 1).await
        .ok_or("Could not get CWD")?;
    assert!(cwd.to_string_lossy().contains("kodegen_test"), "CWD should be in kodegen_test");

    log::info!("  ✅ Terminal state persisted!");

    // Cleanup
    manager.send_input(CONNECTION_ID, 1, "cd /tmp && rm -rf /tmp/kodegen_test\n", false).await?;

    Ok(())
}

async fn test_parallel_terminals(
    manager: &Arc<TerminalManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("SCENARIO 2: Parallel Terminals");
    log::info!("{}", "=".repeat(80));

    let start = Instant::now();

    log::info!("\nLaunching 3 terminals in parallel...");

    // Spawn all terminals in parallel and wait for completion
    let (r1, r2, r3) = tokio::join!(
        spawn_and_wait(manager, CONNECTION_ID, 2, "sleep 2 && echo 'Task 1 complete'"),
        spawn_and_wait(manager, CONNECTION_ID, 3, "echo 'Task 2 complete immediately'"),
        spawn_and_wait(manager, CONNECTION_ID, 4, "sleep 1 && echo 'Task 3 complete'"),
    );

    r1?;
    r2?;
    r3?;

    let elapsed = start.elapsed();
    log::info!("\nCompleted in {:?}", elapsed);
    log::info!("(Sequential: ~3s, Parallel: ~2s)");

    let out1 = manager.get_output(CONNECTION_ID, 2, 0, usize::MAX).await
        .ok_or("Terminal 2 not found")?;
    let out2 = manager.get_output(CONNECTION_ID, 3, 0, usize::MAX).await
        .ok_or("Terminal 3 not found")?;
    let out3 = manager.get_output(CONNECTION_ID, 4, 0, usize::MAX).await
        .ok_or("Terminal 4 not found")?;

    log::info!("\nTerminal 2: {}", out1.lines.join("\n").trim());
    log::info!("Terminal 3: {}", out2.lines.join("\n").trim());
    log::info!("Terminal 4: {}", out3.lines.join("\n").trim());

    assert!(elapsed.as_secs() <= 3, "Should run in parallel (~2s not ~3s)");
    log::info!("✅ Commands ran in parallel!");

    Ok(())
}

async fn test_error_handling(
    manager: &Arc<TerminalManager>,
) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("SCENARIO 3: Error Handling");
    log::info!("{}", "=".repeat(80));

    log::info!("\nStep 1: Run failing command");
    spawn_and_wait(manager, CONNECTION_ID, 5, "cat /this/does/not/exist").await?;

    let output1 = manager.get_output(CONNECTION_ID, 5, 0, usize::MAX).await
        .ok_or("Terminal 5 not found")?;

    log::info!("  Exit code: {:?}", output1.exit_code);
    assert_ne!(output1.exit_code, Some(0), "Should return non-zero exit code");
    log::info!("  ✅ Error properly reported");

    log::info!("\nStep 2: Verify terminal still works after error");
    manager.send_input(CONNECTION_ID, 5, "echo 'Recovered successfully'\n", false).await?;

    let output2 = manager.get_output(CONNECTION_ID, 5, 0, usize::MAX).await
        .ok_or("Terminal 5 not found")?;

    let output_text = output2.lines.join("\n");
    assert!(output_text.contains("Recovered"), "Output should contain success message");
    log::info!("  ✅ Terminal recovered!");

    Ok(())
}

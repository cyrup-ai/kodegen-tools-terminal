//! Example demonstrating all 4 terminal actions: EXEC, READ, LIST, KILL

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL TOOL: ALL 4 ACTIONS DEMO ===\n");

    // Create registry
    let registry = TerminalRegistry::new();

    // Demo 1: EXEC action - Execute commands with default timeout
    demo_exec_action(&registry).await?;
    log::info!("\n{}\n", "=".repeat(80));

    // Demo 2: EXEC action - Background task (await_completion_ms=0)
    demo_background_task(&registry).await?;
    log::info!("\n{}\n", "=".repeat(80));

    // Demo 3: READ action - Check terminal state
    demo_read_action(&registry).await?;
    log::info!("\n{}\n", "=".repeat(80));

    // Demo 4: LIST action - Show all terminals
    demo_list_action(&registry).await?;
    log::info!("\n{}\n", "=".repeat(80));

    // Demo 5: KILL action - Gracefully shutdown terminal
    demo_kill_action(&registry).await?;

    log::info!("\n=== ALL ACTIONS DEMONSTRATED SUCCESSFULLY ===");
    Ok(())
}

/// Demo 1: EXEC action with default timeout (300000ms = 5 minutes)
async fn demo_exec_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("🚀 DEMO 1: EXEC Action (Execute Command)");
    log::info!("   - Default timeout: 300000ms (5 minutes)");
    log::info!("   - Terminal: 0 (default, zero-based indexing)");

    let terminal = registry.find_or_create_terminal("demo-connection", 0).await?;
    let request_id = rmcp::model::RequestId::String("exec-demo-1".to_string().into());

    log::info!("\n   Executing: pwd && ls -la");
    let output = terminal.execute_command(
        request_id,
        "pwd && ls -la".to_string(),
        300_000, // 5 minutes timeout
    ).await?;

    log::info!("   ✅ Command completed:");
    log::info!("      Terminal: {:?}", output.terminal);
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      CWD: {}", output.cwd);
    log::info!("      Duration: {}ms", output.duration_ms);
    log::info!("      Completed: {}", output.completed);
    log::info!("      Output (first 100 chars): {}",
               output.output.chars().take(100).collect::<String>().replace('\n', "\\n"));

    Ok(())
}

/// Demo 2: EXEC action with background task (await_completion_ms=0)
async fn demo_background_task(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("🔥 DEMO 2: EXEC Action (Background Task)");
    log::info!("   - Fire-and-forget: await_completion_ms=0");
    log::info!("   - Terminal: 1 (parallel work)");

    let terminal = registry.find_or_create_terminal("demo-connection", 1).await?;
    let request_id = rmcp::model::RequestId::String("exec-bg-1".to_string().into());

    log::info!("\n   Starting background task: sleep 1 && echo 'Background task complete'");
    let output = terminal.execute_command(
        request_id,
        "sleep 1 && echo 'Background task complete' && pwd".to_string(),
        0, // Fire-and-forget!
    ).await?;

    log::info!("   ✅ Background task started:");
    log::info!("      Terminal: {:?}", output.terminal);
    log::info!("      Exit code: {:?} (None = still running)", output.exit_code);
    log::info!("      Completed: {} (false = running in background)", output.completed);
    log::info!("      Message: {}", output.output.trim());

    // Give it time to complete
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    Ok(())
}

/// Demo 3: READ action - Get current 80x24 VTE buffer snapshot
async fn demo_read_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("📖 DEMO 3: READ Action (Get Current Buffer)");
    log::info!("   - Returns 80x24 VTE buffer snapshot");
    log::info!("   - No command execution");

    // Read terminal:1 (where background task ran)
    let terminal = registry.find_or_create_terminal("demo-connection", 1).await?;
    let output = terminal.read_current_state(1).await?;

    log::info!("\n   ✅ Terminal:1 current state:");
    log::info!("      Terminal: {:?}", output.terminal);
    log::info!("      CWD: {}", output.cwd);
    log::info!("      Completed: {} (READ operation itself)", output.completed);
    log::info!("      Buffer snapshot (first 150 chars): {}",
               output.output.chars().take(150).collect::<String>().replace('\n', "\\n"));

    Ok(())
}

/// Demo 4: LIST action - Show all active terminals
async fn demo_list_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("📋 DEMO 4: LIST Action (Show All Terminals)");
    log::info!("   - Returns snapshots of all active terminals");
    log::info!("   - Filtered by connection_id");

    let output = registry.list_all_terminals("demo-connection").await?;

    log::info!("\n   ✅ Active terminals:");
    log::info!("      Terminal: {:?} (None = LIST response)", output.terminal);
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      Completed: {}", output.completed);
    log::info!("\n   Terminals JSON:");

    // Pretty print the JSON output
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&output.output) {
        log::info!("{}", serde_json::to_string_pretty(&parsed)?);
    }

    Ok(())
}

/// Demo 5: KILL action - Gracefully shutdown terminal
async fn demo_kill_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    log::info!("💀 DEMO 5: KILL Action (Graceful Shutdown)");
    log::info!("   - Cleans up Brush shell, VTE processor, threads, channels");
    log::info!("   - Terminal:1 will be terminated");

    let output = registry.kill_terminal("demo-connection", 1).await?;

    log::info!("\n   ✅ Terminal killed:");
    log::info!("      Terminal: {:?}", output.terminal);
    log::info!("      Exit code: {:?}", output.exit_code);
    log::info!("      Message: {}", output.output);
    log::info!("      Duration: {}ms", output.duration_ms);
    log::info!("      Completed: {}", output.completed);

    // Verify it's gone by trying to list
    let list_output = registry.list_all_terminals("demo-connection").await?;
    log::info!("\n   Verification (LIST after KILL):");
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&list_output.output)
        && let Some(arr) = parsed.as_array()
    {
        log::info!("      Remaining terminals: {}", arr.len());
        log::info!("      ✅ Terminal:1 successfully removed!");
    }

    Ok(())
}

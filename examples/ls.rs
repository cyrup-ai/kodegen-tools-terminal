//! Example demonstrating all 4 terminal actions: EXEC, READ, LIST, KILL
//!
//! This example shows ONLY the Display field output - exactly what MCP agents see.

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== TERMINAL TOOL: ALL 4 ACTIONS DEMO ===");
    println!("Shows ONLY Display field - exactly what agents see\n");

    let registry = TerminalRegistry::new();

    // Demo 1: EXEC action
    demo_exec_action(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    // Demo 2: EXEC background task
    demo_background_task(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    // Demo 3: READ action
    demo_read_action(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    // Demo 4: LIST action
    demo_list_action(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    // Demo 5: lsd --tree
    demo_lsd_tree(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    // Demo 6: Verify COLUMNS and LINES environment variables
    demo_verify_columns(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    // Demo 7: Test long line wrapping
    demo_long_lines(&registry).await?;
    println!("\n{}\n", "=".repeat(80));

    println!("\n💀 CLEANUP: Killing all terminals\n");

    // List before cleanup
    println!("📋 Before cleanup - LIST:");
    let list_before = registry.list_all_terminals("demo-connection").await?;
    println!("{}", list_before.output);

    // Kill terminals
    println!("\n💀 Killing terminal 0...");
    let kill0 = registry.kill_terminal("demo-connection", 0).await?;
    println!("{}", kill0.output);

    println!("\n📋 After killing terminal 0 - LIST:");
    let list_mid = registry.list_all_terminals("demo-connection").await?;
    println!("{}", list_mid.output);

    println!("\n💀 Killing terminal 1...");
    let kill1 = registry.kill_terminal("demo-connection", 1).await?;
    println!("{}", kill1.output);

    println!("\n💀 Killing terminal 2...");
    let kill2 = registry.kill_terminal("demo-connection", 2).await?;
    println!("{}", kill2.output);

    println!("\n📋 After killing all terminals - LIST:");
    let list_after = registry.list_all_terminals("demo-connection").await?;
    println!("{}", list_after.output);

    println!("\n✅ All terminals killed");

    Ok(())
}

/// Demo 1: EXEC action
async fn demo_exec_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("🚀 DEMO 1: EXEC Action");
    println!("Executing: pwd && ls -la\n");
    println!("=== DISPLAY OUTPUT ===");

    let terminal = registry.find_or_create_terminal("demo-connection", 0, None).await?;
    let request_id = rmcp::model::RequestId::String("exec-1".to_string().into());

    let output = terminal.execute_command(
        request_id,
        "pwd && ls -la".to_string(),
        true,
        300_000,
        2000,
        None,
    ).await?;

    println!("{}", output.output);

    Ok(())
}

/// Demo 2: EXEC background task
async fn demo_background_task(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("🔥 DEMO 2: EXEC Action (Background)");
    println!("Executing: sleep 1 && echo 'Background complete' && pwd\n");
    println!("=== DISPLAY OUTPUT ===");

    let terminal = registry.find_or_create_terminal("demo-connection", 1, None).await?;
    let request_id = rmcp::model::RequestId::String("exec-bg".to_string().into());

    let output = terminal.execute_command(
        request_id,
        "sleep 1 && echo 'Background task complete' && pwd".to_string(),
        true,
        0, // Fire-and-forget
        2000,
        None,
    ).await?;

    println!("{}", output.output);

    // Wait for completion
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

    Ok(())
}

/// Demo 3: READ action
async fn demo_read_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("📖 DEMO 3: READ Action");
    println!("Reading terminal 1 buffer\n");
    println!("=== DISPLAY OUTPUT ===");

    let terminal = registry.find_or_create_terminal("demo-connection", 1, None).await?;
    let output = terminal.read_current_state(1, 2000).await?;

    println!("{}", output.output);

    Ok(())
}

/// Demo 4: LIST action
async fn demo_list_action(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("📋 DEMO 4: LIST Action");
    println!("Listing all terminals\n");
    println!("=== DISPLAY OUTPUT ===");

    let output = registry.list_all_terminals("demo-connection").await?;

    println!("{}", output.output);

    Ok(())
}

/// Demo 5: lsd --tree
async fn demo_lsd_tree(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("🌳 DEMO 5: lsd --tree ./src/");
    println!("Executing: cd ... && lsd --tree ./src/\n");
    println!("=== DISPLAY OUTPUT ===");

    let terminal = registry.find_or_create_terminal("demo-connection", 2, None).await?;
    let request_id = rmcp::model::RequestId::String("exec-tree".to_string().into());

    let output = terminal.execute_command(
        request_id,
        "cd /Volumes/samsung_t9/kodegen-workspace/packages/kodegen-native-notify && lsd --tree ./src/".to_string(),
        true,
        30_000,
        2000,
        None,
    ).await?;

    println!("{}", output.output);

    Ok(())
}

/// Demo 6: Verify COLUMNS and LINES environment variables
async fn demo_verify_columns(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("✅ DEMO 6: Verify COLUMNS and LINES");
    println!("Executing: echo \"COLUMNS=$COLUMNS LINES=$LINES\"\n");
    println!("=== DISPLAY OUTPUT ===");

    let terminal = registry.find_or_create_terminal("demo-connection", 3, None).await?;
    let request_id = rmcp::model::RequestId::String("verify-columns".to_string().into());

    let output = terminal.execute_command(
        request_id,
        "echo \"COLUMNS=$COLUMNS\" && echo \"LINES=$LINES\"".to_string(),
        true,
        30_000,
        2000,
        None,
    ).await?;

    println!("{}", output.output);

    Ok(())
}

/// Demo 7: Test long line wrapping
async fn demo_long_lines(registry: &TerminalRegistry) -> Result<(), Box<dyn std::error::Error>> {
    println!("📏 DEMO 7: Test Long Line Wrapping");
    println!("Outputting lines of exactly 40, 80, 100, 120, 200, and 300 characters to test wrapping\n");
    println!("=== DISPLAY OUTPUT ===");

    let terminal = registry.find_or_create_terminal("demo-connection", 4, None).await?;
    let request_id = rmcp::model::RequestId::String("test-wrapping".to_string().into());

    // Create lines of exactly 40, 80, 100, 120, 200, and 300 characters
    let command = r#"
echo "40chars: 0123456789012345678901234567890123456789"
echo "80chars: 01234567890123456789012345678901234567890123456789012345678901234567890123456789"
echo "100chars: 0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"
echo "120chars: 012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"
echo "200chars: 01234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"
echo "300chars: 012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789"
"#;

    let output = terminal.execute_command(
        request_id,
        command.trim().to_string(),
        true,
        30_000,
        2000,
        None,
    ).await?;

    println!("{}", output.output);

    Ok(())
}

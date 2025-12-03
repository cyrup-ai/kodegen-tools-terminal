//! Example demonstrating the clear parameter bug - running multiple commands on terminal:0
//!
//! This demonstrates that when reusing terminal:0, the `clear: true` parameter
//! (which defaults to true) does NOT clear the previous output from the VTE buffer.
//!
//! Expected behavior: Each command should show ONLY its own output
//! Actual behavior: Output accumulates across commands

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== CLEAR PARAMETER BUG DEMONSTRATION ===\n");
    log::info!("Running 3 commands on terminal:0 to show output accumulation\n");
    log::info!("Schema default: clear: true (should clear VTE buffer before each command)");
    log::info!("Actual behavior: Output accumulates because clear parameter is ignored\n");
    log::info!("{}\n", "=".repeat(80));

    // Create registry
    let registry = TerminalRegistry::new();
    let terminal = registry.find_or_create_terminal("clear-demo", 0, None).await?;

    // Command 1: Echo first message
    log::info!("🔵 COMMAND 1 on terminal:0");
    log::info!("   Executing: echo 'FIRST COMMAND OUTPUT'\n");
    
    let request_id_1 = rmcp::model::RequestId::String("cmd-1".to_string().into());
    let output1 = terminal.execute_command(
        request_id_1,
        "echo 'FIRST COMMAND OUTPUT'".to_string(),
        true,   // clear
        30_000, // 30 second timeout
        2000,   // tail: return last 2000 lines
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   ✅ Command 1 output:");
    log::info!("   {}", "─".repeat(78));
    log::info!("{}", output1.output);
    log::info!("   {}", "─".repeat(78));
    log::info!("   Exit code: {:?}, Duration: {}ms\n", output1.exit_code, output1.duration_ms);

    // Small delay for clarity
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Command 2: Echo second message
    log::info!("\n🟢 COMMAND 2 on terminal:0 (REUSING SAME TERMINAL)");
    log::info!("   Expected: Only 'SECOND COMMAND OUTPUT' (if clear worked)");
    log::info!("   Actual: Will also contain 'FIRST COMMAND OUTPUT' (bug)\n");
    log::info!("   Executing: echo 'SECOND COMMAND OUTPUT'\n");

    let request_id_2 = rmcp::model::RequestId::String("cmd-2".to_string().into());
    let output2 = terminal.execute_command(
        request_id_2,
        "echo 'SECOND COMMAND OUTPUT'".to_string(),
        true,   // clear
        30_000,
        2000,
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   ✅ Command 2 output:");
    log::info!("   {}", "─".repeat(78));
    log::info!("{}", output2.output);
    log::info!("   {}", "─".repeat(78));
    log::info!("   Exit code: {:?}, Duration: {}ms\n", output2.exit_code, output2.duration_ms);

    // Check if output contains FIRST command (it shouldn't if clear worked)
    if output2.output.contains("FIRST COMMAND OUTPUT") {
        log::warn!("   ⚠️  BUG CONFIRMED: Output contains previous command!");
        log::warn!("   ⚠️  The 'clear: true' parameter is being ignored!");
    } else {
        log::info!("   ✅ Output is clean (clear parameter worked)");
    }

    // Small delay for clarity
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    // Command 3: Echo third message
    log::info!("\n🟡 COMMAND 3 on terminal:0 (REUSING SAME TERMINAL AGAIN)");
    log::info!("   Expected: Only 'THIRD COMMAND OUTPUT' (if clear worked)");
    log::info!("   Actual: Will contain FIRST + SECOND + THIRD (bug)\n");
    log::info!("   Executing: echo 'THIRD COMMAND OUTPUT'\n");

    let request_id_3 = rmcp::model::RequestId::String("cmd-3".to_string().into());
    let output3 = terminal.execute_command(
        request_id_3,
        "echo 'THIRD COMMAND OUTPUT'".to_string(),
        true,   // clear
        30_000,
        2000,
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   ✅ Command 3 output:");
    log::info!("   {}", "─".repeat(78));
    log::info!("{}", output3.output);
    log::info!("   {}", "─".repeat(78));
    log::info!("   Exit code: {:?}, Duration: {}ms\n", output3.exit_code, output3.duration_ms);

    // Check if output contains FIRST and SECOND commands (it shouldn't if clear worked)
    let contains_first = output3.output.contains("FIRST COMMAND OUTPUT");
    let contains_second = output3.output.contains("SECOND COMMAND OUTPUT");
    
    if contains_first || contains_second {
        log::warn!("\n   ⚠️  BUG CONFIRMED: Output contains previous commands!");
        if contains_first { log::warn!("      - Contains FIRST command output"); }
        if contains_second { log::warn!("      - Contains SECOND command output"); }
        log::warn!("      - The 'clear: true' parameter is completely ignored!");
    } else {
        log::info!("\n   ✅ Output is clean (clear parameter worked)");
    }

    // Cleanup
    log::info!("\n{}\n", "=".repeat(80));
    log::info!("💀 CLEANUP: Killing terminal:0");
    let kill_output = registry.kill_terminal("clear-demo", 0).await?;
    log::info!("   ✅ Terminal killed: exit_code={:?}, duration={}ms", 
        kill_output.exit_code, kill_output.duration_ms);

    log::info!("\n=== DEMONSTRATION COMPLETE ===");
    log::info!("\nSUMMARY:");
    log::info!("  - The 'clear' parameter exists in the schema (default: true)");
    log::info!("  - It's supposed to clear the VTE buffer before each command");
    log::info!("  - But it's completely ignored in the implementation");
    log::info!("  - Result: Output accumulates when reusing terminals\n");

    Ok(())
}

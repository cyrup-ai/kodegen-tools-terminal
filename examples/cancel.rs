//! Example demonstrating cancel functionality
//!
//! This example shows how the terminal handles cancellation when a long-running
//! command is blocking. Command 1 starts `sleep 60 && pwd` which would block
//! for 60 seconds, but Command 2 (`ls -al`) cancels it and runs instead.
//!
//! The key mechanism is CancellationToken from kodegen_bash_shell:
//! - Each command execution gets a fresh CancellationToken
//! - Before new command starts, cancel signal is sent via cancel_tx channel
//! - KodegenInteractiveThread calls token.cancel() to stop the blocked command
//! - The shell's stream() API respects the token and terminates cleanly

use kodegen_tools_terminal::TerminalRegistry;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();

    log::info!("=== TERMINAL CANCEL DEMO ===\n");
    log::info!("This demo shows how a new command cancels a blocked command.");
    log::info!("Command 1: sleep 60 && pwd (would block for 60 seconds)");
    log::info!("Command 2: ls -al (sent while Command 1 is blocking)\n");

    // Create registry
    let registry = TerminalRegistry::new();

    // Get terminal 0
    let terminal = registry.find_or_create_terminal("demo-connection", 0, None).await?;

    // Command 1: Start a long-running command with short timeout
    // We use await_completion_ms=3000 (3 seconds) so we don't wait forever
    log::info!("🚀 COMMAND 1: Starting 'sleep 60 && pwd' with 3s timeout...");
    log::info!("   This command would block for 60 seconds normally.\n");

    let request_id_1 = rmcp::model::RequestId::String("cmd-1-sleep".to_string().into());
    let output1 = terminal.execute_command(
        request_id_1,
        "sleep 60 && pwd".to_string(),
        true,  // clear
        3_000, // 3 second timeout - will timeout since sleep is 60s
        100,
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   Command 1 result:");
    log::info!("      Exit code: {:?} (None = timed out/interrupted)", output1.exit_code);
    log::info!("      Completed: {}", output1.completed);
    log::info!("      Duration: {}ms", output1.duration_ms);
    log::info!("      Output: {}\n", output1.output.lines().take(3).collect::<Vec<_>>().join("\n"));

    // Command 2: Send a new command to the SAME terminal
    // The pre-execution cancel signal will stop the sleeping command
    log::info!("🚀 COMMAND 2: Sending 'ls -al' to same terminal...");
    log::info!("   This triggers a cancel signal to stop the blocked sleep command.\n");

    let request_id_2 = rmcp::model::RequestId::String("cmd-2-ls".to_string().into());
    let output2 = terminal.execute_command(
        request_id_2,
        "ls -al".to_string(),
        true,   // clear
        30_000, // 30 second timeout
        50,     // tail: return last 50 lines
        None,   // ctx: no progress streaming in examples
    ).await?;

    log::info!("   Command 2 result:");
    log::info!("      Exit code: {:?}", output2.exit_code);
    log::info!("      Completed: {}", output2.completed);
    log::info!("      CWD: {}", output2.cwd);
    log::info!("      Duration: {}ms", output2.duration_ms);
    log::info!("      Output:\n{}", output2.output);

    // Cleanup
    log::info!("\n💀 CLEANUP: Killing terminal 0");
    let _ = registry.kill_terminal("demo-connection", 0).await?;
    log::info!("   ✅ Terminal 0 killed");

    log::info!("\n=== CANCEL DEMO COMPLETE ===");
    log::info!("The second command successfully ran despite the first command blocking.");
    log::info!("This is possible because:");
    log::info!("  1. Terminal::execute_command() sends cancel via cancel_tx channel before new command");
    log::info!("  2. KodegenInteractiveThread receives cancel signal and calls token.cancel()");
    log::info!("  3. CancellationToken propagates to shell.stream() which terminates cleanly");
    log::info!("  4. The command loop processes new 'ls -al' command with fresh token");

    Ok(())
}

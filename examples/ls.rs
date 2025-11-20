use kodegen_tools_terminal::TerminalManager;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug"))
        .init();

    log::info!("=== Starting terminal test ===");

    // Create manager
    let terminal_manager = TerminalManager::new();
    
    // 1. Start terminal with ls -al command
    let command = "ls -al";
    log::info!("Starting command: {}", command);
    
    let pid = terminal_manager.spawn_command(command, None).await?;
    log::info!("Got PID: {}", pid);

    // 2. Wait a bit for command to execute
    log::info!("Waiting 500ms for command to produce output...");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // 3. Try reading output multiple times
    for attempt in 1..=5 {
        log::info!("=== Read attempt {} ===", attempt);
        
        let read_result = terminal_manager.get_output(pid, 0, 100).await;
        
        if let Some(output) = read_result {
            log::info!("Read result: total_lines={}, lines_returned={}", 
                       output.total_lines, output.lines.len());
            
            if !output.lines.is_empty() {
                log::info!("Output lines:");
                for (i, line) in output.lines.iter().enumerate() {
                    log::info!("  [{}] {}", i, line);
                }
                break;
            } else {
                log::warn!("No output yet, waiting 200ms...");
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        } else {
            log::error!("Session not found for PID {}", pid);
            break;
        }
    }

    log::info!("=== Test complete ===");
    Ok(())
}

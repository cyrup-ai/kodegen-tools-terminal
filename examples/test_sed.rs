use kodegen_tools_terminal::pty::terminal::Terminal;
use rmcp::model::RequestId;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    env_logger::init();

    println!("=== Testing sed Override ===\n");

    // Create terminal
    let terminal = Terminal::builder()
        .cwd("/tmp")
        .build()
        .await?;

    // Test 1: Try to use sed (should show error)
    println!("Test 1: Running 'sed s/foo/bar/ file.txt'");
    let result = terminal
        .execute_command(
            RequestId::Number(1),
            "sed s/foo/bar/ file.txt".to_string(),
        )
        .await?;

    println!("Exit code: {}", result.exit_code);
    println!("Output:\n{}", result.output);
    println!("\n{}", "=".repeat(80));

    // Test 2: Verify we can still run normal commands
    println!("\nTest 2: Running 'echo Hello World' (should work)");
    let result = terminal
        .execute_command(
            RequestId::Number(2),
            "echo Hello World".to_string(),
        )
        .await?;

    println!("Exit code: {}", result.exit_code);
    println!("Output:\n{}", result.output);

    Ok(())
}

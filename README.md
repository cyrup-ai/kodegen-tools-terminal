# kodegen_tools_terminal

[![License](https://img.shields.io/badge/license-Apache%202.0%20OR%20MIT-blue.svg)](LICENSE.md)
[![Crates.io](https://img.shields.io/crates/v/kodegen_tools_terminal.svg)](https://crates.io/crates/kodegen_tools_terminal)

High-performance terminal management library and MCP (Model Context Protocol) server for code generation agents. Part of the [KODEGEN.ᴀɪ](https://kodegen.ai) ecosystem.

## Features

- 🖥️ **PTY-based Terminal Sessions** - Full pseudo-terminal support with VT100 emulation
- 🎨 **Interactive Command Support** - Run REPLs, editors (vim, nano), and TUI apps (top, htop)
- 📄 **Paginated Output Streaming** - Memory-efficient output retrieval with configurable pagination
- 🔒 **Command Security** - Built-in validation and blocking of dangerous commands
- ⚡ **Async-First Design** - Built on Tokio for high-performance concurrent operations
- 🧹 **Automatic Cleanup** - Background tasks prevent unbounded memory growth
- 🔌 **MCP Protocol** - Expose terminal tools via Model Context Protocol server

## Installation

Add to your `Cargo.toml`:

```toml
[dependencies]
kodegen_tools_terminal = "0.1"
```

Or run:

```bash
cargo add kodegen_tools_terminal
```

## Quick Start

### As a Library

```rust
use kodegen_tools_terminal::{TerminalManager, CommandManager};
use std::sync::Arc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create terminal manager
    let manager = Arc::new(TerminalManager::new());

    // Start background cleanup task
    manager.clone().start_cleanup_task();

    // Execute a command
    let result = manager.execute_command(
        "echo 'Hello, World!'",
        Some(100), // 100ms initial delay
        None       // default shell
    ).await?;

    println!("PID: {}", result.pid);
    println!("Output: {}", result.output);

    // Get paginated output
    if let Some(output) = manager.get_output(result.pid, 0, 100).await {
        for line in output.lines {
            println!("{}", line);
        }
    }

    Ok(())
}
```

### As an MCP Server

Run the HTTP/SSE server:

```bash
cargo run --bin kodegen-terminal
```

The server exposes 5 MCP tools for terminal management (see [MCP Tools](#mcp-tools) below).

## MCP Tools

### 1. `start_terminal_command`

Start a new terminal session with a command.

**Arguments:**
```json
{
  "command": "python3 -i",
  "initial_delay_ms": 100,
  "shell": "/bin/bash"  // optional
}
```

**Returns:**
```json
{
  "pid": 1000,
  "output": "Python 3.x.x\n>>> ",
  "is_blocked": false,
  "ready_for_input": true
}
```

### 2. `read_terminal_output`

Read paginated output from a running session.

**Arguments:**
```json
{
  "pid": 1000,
  "offset": 0,     // negative for tail behavior
  "length": 100    // max lines to return
}
```

**Returns:**
```json
{
  "pid": 1000,
  "lines": ["line1", "line2", "..."],
  "total_lines": 250,
  "lines_returned": 100,
  "is_complete": false,
  "has_more": true
}
```

### 3. `send_terminal_input`

Send input to an interactive session.

**Arguments:**
```json
{
  "pid": 1000,
  "input": "print('hello')",
  "append_newline": true  // default: true
}
```

### 4. `stop_terminal_command`

Terminate a running session.

**Arguments:**
```json
{
  "pid": 1000
}
```

### 5. `list_terminal_commands`

List all active terminal sessions.

**Returns:**
```json
{
  "sessions": [
    {
      "pid": 1000,
      "is_blocked": false,
      "runtime": 5420  // milliseconds
    }
  ]
}
```

## Interactive Session Example

```rust
use kodegen_tools_terminal::TerminalManager;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let manager = Arc::new(TerminalManager::new());
    manager.clone().start_cleanup_task();

    // Start Python REPL
    let result = manager.execute_command("python3 -i", Some(500), None).await?;
    let pid = result.pid;

    // Send Python code
    manager.send_input(pid, "x = 42", true).await?;
    sleep(Duration::from_millis(100)).await;

    manager.send_input(pid, "print(x * 2)", true).await?;
    sleep(Duration::from_millis(100)).await;

    // Read output
    if let Some(output) = manager.get_output(pid, -20, 20).await {
        for line in output.lines {
            println!("{}", line);
        }
    }

    // Cleanup
    manager.force_terminate(pid).await?;

    Ok(())
}
```

## Command Security

The `CommandManager` provides security controls by blocking dangerous commands:

```rust
use kodegen_tools_terminal::CommandManager;

let cmd_manager = CommandManager::with_defaults();

// Blocked commands include:
// - Destructive: rm, dd, shred, truncate
// - Privilege: sudo, su, doas
// - Network: wget, curl, nc, ssh
// - System: reboot, shutdown, kill

assert!(!cmd_manager.validate_command("sudo rm -rf /"));
assert!(cmd_manager.validate_command("ls -la"));
```

Custom blocked commands:

```rust
let cmd_manager = CommandManager::new(vec![
    "custom_dangerous_cmd".to_string(),
]);
```

## Architecture Highlights

### PTY vs Standard Pipes

This library uses **pseudo-terminals (PTY)** instead of standard pipes:

- ✅ TTY detection works (programs behave as if in a real terminal)
- ✅ ANSI color sequences preserved
- ✅ Interactive programs supported (vim, less, top)
- ✅ Proper line wrapping and cursor control

### Memory Efficiency

- **Paginated reads**: Only requested line ranges loaded into memory
- **VT100 scrollback**: Configurable buffer (default: 10,000 lines)
- **Automatic cleanup**: Sessions cleaned up after inactivity (5 minutes for active, 30 seconds for completed)

### Session Management

- **Synthetic PIDs**: Internal tracking independent of OS PIDs
- **Session limits**: Maximum 100 concurrent sessions
- **Two-tier cleanup**: completed → archived → deleted
- **Background cleanup task**: Runs every 60 seconds

### REPL Detection

Automatic detection of REPL prompts:
- Python: `>>> `, `... `
- Bash/Zsh: `$ `, `# `
- Node.js: `> `, `node> `
- IPython: `In [N]: `
- And more...

When detected, sets `ready_for_input: true` in response.

## Development

```bash
# Build
cargo build

# Run tests
cargo test

# Run example
cargo run --example terminal_demo

# Run MCP server
cargo run --bin kodegen-terminal

# Build release
cargo build --release
```

## Examples

See [`examples/terminal_demo.rs`](examples/terminal_demo.rs) for comprehensive usage examples including:
- Starting and stopping commands
- Interactive bash sessions with command history
- Dynamic output (top command)
- Multi-step REPL interactions

## Dependencies

Core dependencies:
- [`tokio`](https://tokio.rs/) - Async runtime
- [`portable-pty`](https://crates.io/crates/portable-pty) - Cross-platform PTY
- [`vt100`](https://crates.io/crates/vt100) - VT100 terminal emulator
- [`rmcp`](https://crates.io/crates/rmcp) - MCP SDK

## Contributing

Contributions welcome! Please ensure:
- Code follows Rust conventions
- Tests pass: `cargo test`
- New features include tests
- Security-sensitive changes reviewed carefully

## License

Dual-licensed under Apache 2.0 OR MIT. See [LICENSE.md](LICENSE.md) for details.

## Links

- **Homepage**: [https://kodegen.ai](https://kodegen.ai)
- **Repository**: [https://github.com/cyrup-ai/kodegen-tools-terminal](https://github.com/cyrup-ai/kodegen-tools-terminal)
- **MCP Protocol**: [https://modelcontextprotocol.io](https://modelcontextprotocol.io)

---

**KODEGEN.ᴀɪ** - Memory-efficient, Blazing-Fast, MCP tools for code generation agents.

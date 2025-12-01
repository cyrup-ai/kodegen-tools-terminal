<div align="center">
  <img src="assets/img/banner.png" alt="Kodegen AI Banner" width="100%" />
</div>

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

Execute shell commands with full terminal emulation. Supports long-running commands, output streaming, and session management.

**Primary use cases:**
- Build/compile commands (30s-5min): check, build, test, install
- Development servers and file watchers
- Interactive programs (REPLs, editors, monitoring tools)

**Arguments:**
```json
{
  "command": "python3 -i",
  "initial_delay_ms": 100,  // Window for fast commands to complete (default: 100ms)
  "shell": "/bin/bash"      // optional
}
```

**Returns (always returns PID + output + status):**
```json
{
  "pid": 1000,
  "output": "Python 3.x.x\n>>> ",  // Captured during initial_delay_ms
  "still_running": false,           // true if command still executing
  "ready_for_input": true
}
```

**Return behavior:**
- Fast commands (<100ms): Returns complete output, `still_running: false`
- Slow commands (30s+): Returns partial output, `still_running: true` → use `read_terminal_output(pid)` to poll

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
      "still_running": false,
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

## Command Validation System

The terminal includes a comprehensive **context-aware validation system** that blocks dangerous operations while allowing safe commands. Validation happens automatically before command execution.

### Architecture

```text
Terminal::execute_command()
         ↓
ValidationEngine::validate()
         ↓
    [Rule Lookup]
         ↓
   [Analyzers]
    ↓         ↓
FlagAnalyzer  PathAnalyzer
         ↓
[ValidationDecision]
```

### Default Security Rules

The validation engine comes with hardcoded security rules covering five categories:

#### 1. Always Blocked Commands
Never allowed due to security risks:
- **Privilege escalation**: `sudo`, `su`, `doas`
- **System control**: `reboot`, `shutdown`, `halt`, `poweroff`

#### 2. Destructive Commands
Pattern-based restrictions for data-modifying commands:
- **File deletion**: `rm`, `rmdir` (blocks `-rf`, `-fr`, system paths)
- **Disk operations**: `dd`, `shred`, `wipe`
- **Disk formatting**: `mkfs.*`, `fdisk`, `parted`

#### 3. Permission Commands
Access control modification:
- **File permissions**: `chmod`, `chown`, `chgrp`
- **Extended attributes**: `chattr`, `setfacl`

#### 4. System Modification
Configuration changes:
- **Firewall**: `iptables`, `ufw`, `firewall-cmd`
- **Services**: `systemctl`, `service`
- **Filesystems**: `mount`, `umount`

#### 5. Package Management
Software installation:
- **System packages**: `apt`, `yum`, `dnf`, `pacman`
- **Language packages**: `npm`, `pip`, `gem`, `cargo`

### Basic Usage

Validation is automatic - no explicit calls needed:

```rust
use kodegen_tools_terminal::pty::terminal::Terminal;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Create terminal (ValidationEngine initialized automatically)
    let terminal = Terminal::builder().build().await?;

    // Safe commands are allowed
    let result = terminal.execute_command(
        request_id,
        "ls -la".to_string(),
        5000
    ).await?;

    // Dangerous commands are blocked
    let result = terminal.execute_command(
        request_id,
        "rm -rf /".to_string(),
        5000
    ).await?;
    // Returns error with educational message about using MCP tools

    Ok(())
}
```

### Programmatic Customization

Add custom validation rules programmatically:

```rust
use kodegen_tools_terminal::validation::{ValidationEngine, CommandRule, ViolationType};
use std::borrow::Cow;

// Access the terminal's validation engine
let engine = &terminal.validation_engine;

// Add custom rule with builder pattern
let rule = CommandRule::builder("mycmd")
    .default_allow(true)
    .block_pattern(
        Cow::Borrowed(r"--dangerous-flag"),
        ViolationType::DangerousFlag,
        "This flag is dangerous"
    )
    .restricted_path("/sensitive/data")
    .build();

engine.add_rule(rule);

// Add always-blocked command
engine.add_rule(CommandRule::always_blocked("forbidden-cmd"));
```

### Validation Decision Types

Commands are either allowed or blocked:

```rust
use kodegen_tools_terminal::validation::{ValidationDecision, ViolationType};

match engine.validate("rm -rf /") {
    ValidationDecision::Allow => {
        // Command is safe to execute
    }
    ValidationDecision::Block { reason, violation_type } => {
        // Command blocked with explanation
        match violation_type {
            ViolationType::AlwaysBlocked => { /* Never allowed */ }
            ViolationType::DangerousFlag => { /* Dangerous flag detected */ }
            ViolationType::RestrictedPath => { /* Restricted path access */ }
        }
    }
}
```

### Educational Builtins

Some Unix commands are intercepted before validation to guide users toward MCP tools:

- `find` → Use `fs_search` tool (10-100x faster)
- `grep` → Use `fs_search` tool with content search
- `mv` → Use `fs_move_file` tool
- `chmod`, `chown` → Not needed (MCP tools handle permissions)
- `ln` → Use `fs_move_file` or `fs_write_file`
- `kill`, `killall`, `pkill` → Use `process_kill` tool

These intercepts provide friendly educational messages with examples.

### Performance

- **Rule lookup**: O(1) average case (DashMap concurrent hashmap)
- **Pre-compiled patterns**: Regex patterns compiled once at startup
- **Zero-allocation validation**: Reuses existing allocations
- **Thread-safe**: All operations safe for concurrent access
- **Open-world**: Unknown commands allowed by default (no overhead)

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

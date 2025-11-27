//! Educational builtin for kill command
//!
//! This builtin intercepts the `kill` command and redirects users to use
//! the safer MCP process management tools instead.
//!
//! # Why This Exists
//!
//! The traditional `kill` command is:
//! - **Dangerous** - can accidentally terminate wrong processes (PID reuse)
//! - **No validation** - can kill system critical processes
//! - **Brutal** - SIGKILL (-9) prevents cleanup and can corrupt state
//! - **No ownership checks** - root can kill anything
//!
//! The MCP process tools are:
//! - **Safe** (ownership validation, system process protection)
//! - **Explicit** (list processes first, review, then terminate)
//! - **Graceful** (SIGTERM by default for clean shutdown)
//! - **Clear errors** (explains why termination failed)
//!
//! # How It Works
//!
//! When a user types `kill ...`, this builtin:
//! 1. Intercepts the command before shell execution
//! 2. Writes an educational message to stderr
//! 3. Returns exit code 1 (failure)
//!
//! This guides users toward the safer MCP tool workflow.

use brush_core::builtins::Command;
use brush_core::commands::ExecutionContext;
use brush_core::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// kill override - redirects to kodegen process tools
#[derive(Parser)]
pub struct KillCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for KillCommand {
    type Error = Error;

    #[allow(clippy::manual_async_fn)]
    fn execute(
        &self,
        context: ExecutionContext<'_>,
    ) -> impl std::future::Future<Output = Result<ExecutionResult, Self::Error>>
    + std::marker::Send {
        async move {
            writeln!(
                context.stderr(),
                "Error: 'kill' is not available in this shell.\n\n\
                 Instead, use KODEGEN's process management tools:\n\n\
                 • process_list - List running processes with filtering\n\
                   - Filter by name or PID\n\
                   - See CPU/memory usage\n\
                   - Safe, read-only operation\n\
                   - Returns process metadata (owner, command, etc.)\n\n\
                 • process_kill - Terminate processes safely\n\
                   - Validates process ownership (can't kill other users' processes)\n\
                   - Prevents killing system critical processes (PID 1, kernel threads)\n\
                   - Graceful termination (SIGTERM by default)\n\
                   - Clear error messages if termination fails\n\n\
                 Common use cases:\n\n\
                 1. Find and kill a stuck process:\n\
                    processes = process_list(filter: \"myapp\")\n\
                    # Examine output to find PID\n\
                    process_kill(pid: 12345)\n\n\
                 2. Kill process by exact name:\n\
                    processes = process_list(filter: \"python script.py\")\n\
                    # Kill the specific process\n\
                    process_kill(pid: processes[0].pid)\n\n\
                 3. Terminate child process:\n\
                    # After spawning a process in terminal\n\
                    processes = process_list(filter: \"child_process\")\n\
                    process_kill(pid: child_pid)\n\n\
                 Safety guarantees that raw 'kill' cannot provide:\n\n\
                 ✓ Validates you own the process before terminating\n\
                 ✓ Prevents accidental termination of system processes\n\
                 ✓ Cannot kill init (PID 1) or kernel threads\n\
                 ✓ Cannot kill processes owned by other users\n\
                 ✓ Graceful shutdown (SIGTERM) instead of force kill (SIGKILL)\n\
                 ✓ Clear error messages explaining why termination failed\n\n\
                 Why raw 'kill' is dangerous:\n\
                 - Can accidentally kill wrong PID (PIDs get reused)\n\
                 - No ownership validation (root can kill anything)\n\
                 - SIGKILL (-9) prevents cleanup (corrupted state, lost data)\n\
                 - Easy to kill system critical processes\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

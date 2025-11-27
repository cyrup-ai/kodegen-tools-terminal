//! Educational builtin for killall command
//!
//! This builtin intercepts the `killall` command and redirects users to use
//! the safer MCP process management tools for bulk termination.
//!
//! # Why This Exists
//!
//! The traditional `killall` command is:
//! - **Overly broad** - kills ALL processes matching name (including system processes)
//! - **No confirmation** - terminates immediately without review
//! - **Cross-platform inconsistent** - different behavior on macOS vs Linux
//! - **Dangerous** - can accidentally kill critical system services
//!
//! The MCP process tools are:
//! - **Explicit** (review process list before terminating)
//! - **Controlled** (iterate and kill one by one with error handling)
//! - **Safe** (cannot match system processes you don't own)
//! - **Predictable** (same behavior across all platforms)
//!
//! # How It Works
//!
//! When a user types `killall ...`, this builtin:
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

/// killall override - redirects to kodegen process tools
#[derive(Parser)]
pub struct KillallCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for KillallCommand {
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
                "Error: 'killall' is not available in this shell.\n\n\
                 Instead, use KODEGEN's process management tools for safer bulk termination:\n\n\
                 • process_list - Find all processes matching criteria\n\
                   - Filter by name (exact or partial match)\n\
                   - Filter by PID range\n\
                   - See which processes will be affected\n\
                   - Returns detailed process information\n\n\
                 • process_kill - Terminate processes individually\n\
                   - Validates each process before termination\n\
                   - Skips system processes automatically\n\
                   - Provides per-process success/failure status\n\n\
                 Common use cases:\n\n\
                 1. Kill all instances of an application:\n\
                    processes = process_list(filter: \"chrome\")\n\
                    # Review which processes will be killed\n\
                    for process in processes:\n\
                        process_kill(pid: process.pid)\n\n\
                 2. Clean up stuck worker processes:\n\
                    processes = process_list(filter: \"worker_\")\n\
                    # Kill workers but not the manager\n\
                    for process in processes:\n\
                        if \"worker_\" in process.name:\n\
                            process_kill(pid: process.pid)\n\n\
                 3. Terminate all processes from a script:\n\
                    processes = process_list(filter: \"my_script.py\")\n\
                    for process in processes:\n\
                        process_kill(pid: process.pid)\n\n\
                 Why this is safer than 'killall':\n\n\
                 ✓ Review processes BEFORE killing (no surprises)\n\
                 ✓ Explicit iteration (you control which processes die)\n\
                 ✓ Per-process error handling (some succeed, some fail)\n\
                 ✓ Cannot accidentally match system processes\n\
                 ✓ Cannot kill processes you don't own\n\n\
                 Why raw 'killall' is dangerous:\n\
                 - Overly broad matching (killall python kills ALL python processes)\n\
                 - No confirmation (kills immediately)\n\
                 - Can kill system services by accident\n\
                 - Different behavior on different OSes (macOS vs Linux)\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

//! Educational builtin for pkill command
//!
//! This builtin intercepts the `pkill` command and redirects users to use
//! the safer MCP process management tools for pattern-based termination.
//!
//! # Why This Exists
//!
//! The traditional `pkill` command is:
//! - **Pattern matching too broad** - can match unrelated processes
//! - **No preview** - kills immediately without showing matches
//! - **Regex inconsistent** - different regex support across systems
//! - **Dangerous** - can accidentally match and kill system processes
//!
//! The MCP process tools are:
//! - **Explicit** (preview matches before killing)
//! - **Rich filtering** (CPU, memory, start time, command args)
//! - **Controlled** (explicit control over which processes die)
//! - **Cross-platform** (same behavior on all OSes)
//!
//! # How It Works
//!
//! When a user types `pkill ...`, this builtin:
//! 1. Intercepts the command before shell execution
//! 2. Writes an educational message to stderr
//! 3. Returns exit code 1 (failure)
//!
//! This guides users toward the safer MCP tool workflow.

use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// pkill override - redirects to kodegen process tools
#[derive(Parser)]
pub struct PkillCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for PkillCommand {
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
                "Error: 'pkill' is not available in this shell.\n\n\
                 Instead, use KODEGEN's process management tools for pattern-based termination:\n\n\
                 • process_list - Find processes by pattern matching\n\
                   - Filter by partial name match\n\
                   - Filter by command line arguments\n\
                   - Preview matches before taking action\n\
                   - Sort by CPU/memory to find problematic processes\n\n\
                 • process_kill - Terminate matched processes\n\
                   - Validates ownership and permissions\n\
                   - Graceful termination by default\n\
                   - Individual process control\n\n\
                 Common use cases:\n\n\
                 1. Kill processes by name pattern:\n\
                    processes = process_list(filter: \"worker\")\n\
                    # Review matches\n\
                    for process in processes:\n\
                        process_kill(pid: process.pid)\n\n\
                 2. Kill processes consuming high CPU:\n\
                    processes = process_list()\n\
                    # Sort by CPU usage\n\
                    high_cpu = [p for p in processes if p.cpu_percent > 80]\n\
                    for process in high_cpu:\n\
                        process_kill(pid: process.pid)\n\n\
                 3. Kill processes by command line argument:\n\
                    processes = process_list()\n\
                    # Filter by command line\n\
                    matching = [p for p in processes if \"--worker\" in p.command]\n\
                    for process in matching:\n\
                        process_kill(pid: process.pid)\n\n\
                 4. Kill oldest process matching pattern:\n\
                    processes = process_list(filter: \"myapp\")\n\
                    # Sort by start time (oldest first)\n\
                    if processes:\n\
                        process_kill(pid: processes[0].pid)\n\n\
                 Advantages over 'pkill':\n\n\
                 ✓ Preview matches before killing (no surprises)\n\
                 ✓ Rich filtering (CPU, memory, start time, command args)\n\
                 ✓ Explicit control over which processes die\n\
                 ✓ Per-process error handling and logging\n\
                 ✓ Cannot accidentally match unrelated processes\n\
                 ✓ Cross-platform consistency (same behavior on all OSes)\n\n\
                 Why raw 'pkill' is dangerous:\n\
                 - Pattern matching can be overly broad\n\
                 - No preview (kills immediately)\n\
                 - Regex syntax varies by implementation\n\
                 - Can match system processes by accident\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

//! Educational builtin for mv command
//!
//! This builtin intercepts the `mv` command and redirects users to use
//! the safer MCP `fs_move_file` tool instead.
//!
//! # Why This Exists
//!
//! The traditional `mv` command is:
//! - **Dangerous** - can overwrite files without confirmation
//! - **Not atomic** - can fail mid-operation leaving files in inconsistent state
//! - **Not sandboxed** - can move files to restricted system directories
//! - **No validation** - can accidentally move critical system files
//!
//! The `fs_move_file` MCP tool is:
//! - **Safe** (atomic operations with validation)
//! - **Validated** (checks source and destination paths)
//! - **Sandboxed** (respects allowed directories)
//! - **Integrated** (returns structured results to agents)
//!
//! # How It Works
//!
//! When a user types `mv ...`, this builtin:
//! 1. Intercepts the command before shell execution
//! 2. Writes an educational message to stderr
//! 3. Returns exit code 1 (failure)
//!
//! This guides users toward the safer MCP tool.

use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// mv override - redirects to kodegen filesystem tools
///
/// This command always fails with an educational message pointing users
/// to the `fs_move_file` MCP tool.
///
/// # Arguments
///
/// All arguments are accepted but ignored (via `trailing_var_arg = true`).
///
/// # Returns
///
/// Always returns exit code 1 with an educational message on stderr.
#[derive(Parser)]
pub struct MvCommand {
    /// All arguments (ignored - for compatibility only)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for MvCommand {
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
                "Error: 'mv' is not available in this shell.\n\n\
                 Instead, use KODEGEN's fs_move_file tool for safe atomic moves:\n\n\
                 • fs_move_file - Move or rename files and directories safely\n\
                   - Atomic operations (source and destination in single operation)\n\
                   - Cross-platform (macOS, Linux, Windows)\n\
                   - Validates both source and destination paths\n\
                   - Prevents accidental overwrites of system files\n\n\
                 Common use cases:\n\n\
                 1. Rename a file:\n\
                    fs_move_file(source: \"old_name.txt\", destination: \"new_name.txt\")\n\n\
                 2. Move file to directory:\n\
                    fs_move_file(source: \"file.txt\", destination: \"./backup/file.txt\")\n\n\
                 3. Rename directory:\n\
                    fs_move_file(source: \"old_dir\", destination: \"new_dir\")\n\n\
                 4. Move across directories:\n\
                    fs_move_file(source: \"src/old.rs\", destination: \"backup/old.rs\")\n\n\
                 Safety features:\n\
                 - Validates source file exists before attempting move\n\
                 - Checks destination path is writable\n\
                 - Prevents moves to system directories (/etc, /sys, etc.)\n\
                 - Atomic operation (no partial moves on failure)\n\
                 - Works across filesystems (copies + deletes if needed)\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

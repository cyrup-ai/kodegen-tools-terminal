//! Educational builtin for find command
//!
//! This builtin intercepts the `find` command and redirects users to use
//! the blazing-fast MCP `fs_search` tool instead.
//!
//! # Why This Exists
//!
//! The traditional `find` command is:
//! - **10-100x slower** than `fs_search` (which uses ripgrep internally)
//! - **Dangerous** with flags like `-exec` and `-delete`
//! - **Not sandboxed** - can access any filesystem path
//!
//! The `fs_search` MCP tool is:
//! - **Blazing fast** (built on ripgrep)
//! - **Safe** (no command execution)
//! - **Sandboxed** (respects allowed directories)
//! - **Feature-rich** (regex, glob patterns, file type filtering)
//!
//! # How It Works
//!
//! When a user types `find ...`, this builtin:
//! 1. Intercepts the command before shell execution
//! 2. Writes an educational message to stderr
//! 3. Returns exit code 1 (failure)
//!
//! This guides users toward the safer, faster MCP tool.

use brush_core::builtins::Command;
use brush_core::commands::ExecutionContext;
use brush_core::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// find override - redirects to kodegen filesystem tools
///
/// This command always fails with an educational message pointing users
/// to the `fs_search` MCP tool.
///
/// # Arguments
///
/// All arguments are accepted but ignored (via `trailing_var_arg = true`).
///
/// # Returns
///
/// Always returns exit code 1 with an educational message on stderr.
#[derive(Parser)]
pub struct FindCommand {
    /// All arguments (ignored - for compatibility only)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for FindCommand {
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
                "Error: 'find' is not available in this shell.\n\n\
                 Instead, use KODEGEN's fs_search tool for blazing-fast file searching:\n\n\
                 • fs_search - 10-100x faster than find, respects .gitignore automatically\n\
                   - Full regex support with multiple engines (Rust, PCRE2)\n\
                   - Search file contents AND filenames\n\
                   - Advanced filtering (glob patterns, file types, size limits)\n\
                   - Context lines (before/after match)\n\
                   - Built on ripgrep for maximum performance\n\n\
                 Common use cases:\n\n\
                 1. Find files by name:\n\
                    fs_search(path: \".\", pattern: \".*\\.rs$\", search_in: \"filenames\")\n\n\
                 2. Find files containing text:\n\
                    fs_search(path: \".\", pattern: \"TODO\", output_mode: \"content\")\n\n\
                 3. Search with glob filter:\n\
                    fs_search(path: \"src\", pattern: \"fn main\", file_pattern: \"*.rs\")\n\n\
                 4. Find recently modified files:\n\
                    fs_search(path: \".\", pattern: \".*\", search_in: \"filenames\", sort_by: \"modified\")\n\n\
                 fs_search is purpose-built for agents:\n\
                 - Memory-efficient streaming (handles massive codebases)\n\
                 - Structured output (JSON-parseable results)\n\
                 - No dangerous -exec or -delete flags\n\
                 - Cross-platform (works on macOS, Linux, Windows)\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

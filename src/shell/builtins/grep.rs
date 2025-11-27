//! Educational builtin for grep command
//!
//! This builtin intercepts the `grep` command and redirects users to use
//! the blazing-fast MCP `fs_search` tool instead.
//!
//! # Why This Exists
//!
//! The traditional `grep` command is:
//! - **10-100x slower** than `fs_search` (which uses ripgrep internally)
//! - **Limited** to basic pattern matching
//! - **Not integrated** with MCP tool ecosystem
//!
//! The `fs_search` MCP tool is:
//! - **Blazing fast** (built on ripgrep - the fastest grep alternative)
//! - **Feature-rich** (regex, glob patterns, context lines, multiline)
//! - **Integrated** (returns structured results to agents)
//! - **Respects .gitignore** automatically
//!
//! # How It Works
//!
//! When a user types `grep ...`, this builtin:
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

/// grep override - redirects to kodegen filesystem tools
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
pub struct GrepCommand {
    /// All arguments (ignored - for compatibility only)
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for GrepCommand {
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
                "Error: 'grep' is not available in this shell.\n\n\
                 Instead, use KODEGEN's fs_search tool with powerful regex support:\n\n\
                 • fs_search - Built on ripgrep, 10-100x faster than grep\n\
                   - Full regex support (Rust engine + PCRE2 for advanced patterns)\n\
                   - Case-insensitive, smart-case, word boundaries\n\
                   - Multiline matching with dotall mode\n\
                   - Context lines (before/after/around matches)\n\
                   - Automatically respects .gitignore\n\n\
                 Common use cases:\n\n\
                 1. Search for pattern in files:\n\
                    fs_search(path: \"src\", pattern: \"error\", output_mode: \"content\")\n\n\
                 2. Case-insensitive search:\n\
                    fs_search(path: \".\", pattern: \"TODO\", case_mode: \"insensitive\")\n\n\
                 3. Search with context lines:\n\
                    fs_search(path: \".\", pattern: \"fn main\", context: 3)\n\n\
                 4. Search specific file types:\n\
                    fs_search(path: \".\", pattern: \"import\", type: [\"py\", \"js\"])\n\n\
                 5. Get only file paths (no content):\n\
                    fs_search(path: \".\", pattern: \"test\", return_only: \"paths\")\n\n\
                 6. Count matches per file:\n\
                    fs_search(path: \".\", pattern: \"TODO\", return_only: \"counts\")\n\n\
                 Advantages over grep:\n\
                 - Memory-efficient (streams results, handles huge codebases)\n\
                 - Faster (SIMD optimizations, parallel search)\n\
                 - Smarter (respects .gitignore, .ignore files)\n\
                 - Structured output (JSON-parseable)\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

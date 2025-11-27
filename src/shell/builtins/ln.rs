//! Educational builtin for ln command
//!
//! This builtin intercepts the `ln` command and explains why symlinks
//! aren't supported in the KODEGEN shell environment.
//!
//! # Why Symlinks Are Disabled
//!
//! Symlinks add unnecessary complexity to path management and can create
//! unintended consequences:
//!
//! **Path Validation Complexity**: When symlinks exist, validating paths becomes
//! more complex because you need to resolve links to understand actual file locations.
//!
//! **Mental Model Clarity**: Direct file operations are easier to understand and
//! reason about than operations involving symbolic indirection.
//!
//! **Example of Complexity**:
//! ```bash
//! # Create symlink in workspace
//! cd /home/user/workspace
//! ln -s /etc/passwd ./data.txt
//!
//! # Now when accessing ./data.txt:
//! # - Link location: /home/user/workspace/data.txt (in workspace)
//! # - Actual target: /etc/passwd (system file)
//! # This indirection makes it harder to track what's being accessed
//! ```

use brush_core::builtins::Command;
use brush_core::commands::ExecutionContext;
use brush_core::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// ln override - explains why symlinks aren't supported
#[derive(Parser)]
pub struct LnCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for LnCommand {
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
                "Error: 'ln' is not available in this shell.\n\n\
                 Symbolic links add complexity that makes file operations harder to reason about.\n\n\
                 Instead, use KODEGEN's filesystem tools for clearer alternatives:\n\n\
                 • fs_move_file - Move or rename files atomically\n\
                   - Direct file operations (no indirection)\n\
                   - Clear, explicit file locations\n\
                   - Works across filesystems\n\n\
                 • fs_write_file + fs_read_file - Duplicate file contents\n\
                   - Independent copies (changes don't affect both)\n\
                   - No link breakage if source moves/deletes\n\
                   - Explicit about file relationships\n\n\
                 Common ln use cases and alternatives:\n\n\
                 1. Multiple files with same content:\n\
                    Instead of: ln -s original.txt link.txt\n\
                    Use: content = fs_read_file(\"original.txt\")\n\
                         fs_write_file(\"link.txt\", content)\n\n\
                 2. Organizing files into logical locations:\n\
                    Instead of: ln -s /path/to/file.txt ./file.txt\n\
                    Use: fs_move_file(source, destination)\n\
                    Or: Keep reference to original path in your code/config\n\n\
                 3. Creating aliases for executables:\n\
                    Instead of: ln -s /usr/bin/python3 ./python\n\
                    Use: Execute commands with full paths via terminal tool\n\n\
                 4. Development workflows (node_modules, etc.):\n\
                    Use package manager's built-in mechanisms (npm link, etc.)\n\
                    These handle symlinks safely within their ecosystems\n\n\
                 Why we don't support symlinks:\n\
                 - Adds indirection that makes path validation more complex\n\
                 - Can break when source file moves/deletes (dangling links)\n\
                 - Makes it harder to understand actual file locations\n\
                 - Direct file operations are simpler and clearer\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

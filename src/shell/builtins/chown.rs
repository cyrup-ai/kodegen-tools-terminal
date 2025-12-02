//! Educational builtin for chown command
//!
//! This builtin intercepts the `chown` command and explains why ownership
//! changes are rarely needed in modern deployments.
//!
//! # Why chown is Not Supported
//!
//! **Requires elevated privileges**: The `chown` command needs root access (CAP_CHOWN
//! capability on Linux). Most agent processes run as regular users without these privileges.
//!
//! **Rarely needed in practice**: In containerized and orchestrated environments, files
//! are automatically owned by the running user. Ownership is managed at the infrastructure
//! level, not at runtime.
//!
//! **Simpler mental model**: Focus on file content and organization rather than ownership
//! bits. When agents create files via MCP tools, they're automatically owned correctly.

use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// chown override - educates about ownership management
#[derive(Parser)]
pub struct ChownCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for ChownCommand {
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
                "Error: 'chown' is not available in this shell.\n\n\
                 File ownership changes require elevated privileges and are rarely needed:\n\n\
                 Understanding file ownership:\n\n\
                 • In modern deployments (containers, sandboxes):\n\
                   - Files are owned by the running user automatically\n\
                   - Ownership is managed at the container/system level\n\
                   - No ownership changes needed for normal operations\n\n\
                 • In traditional systems:\n\
                   - Only root can change file ownership (requires CAP_CHOWN)\n\
                   - Agents typically run as non-root users\n\
                   - Ownership changes are administrative tasks\n\n\
                 Alternative approaches:\n\n\
                 1. Creating new files:\n\
                    Use fs_write_file - automatically owned by running user\n\n\
                 2. Moving files:\n\
                    Use fs_move_file - preserves or updates ownership appropriately\n\n\
                 3. Copying files:\n\
                    Read with fs_read_file, write with fs_write_file\n\
                    New file will have correct ownership automatically\n\n\
                 4. In containers:\n\
                    Adjust Dockerfile USER directive to run as desired user\n\
                    All files created by that user will have correct ownership\n\n\
                 If ownership changes are truly required:\n\
                 - Run the entire MCP server as the target user\n\
                 - Or handle ownership in your deployment/provisioning scripts\n\
                 - Agents should work with files they own, not change ownership\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

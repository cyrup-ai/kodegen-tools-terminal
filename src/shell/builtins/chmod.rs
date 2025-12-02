//! Educational builtin for chmod command
//!
//! This builtin intercepts the `chmod` command and explains why permission
//! changes are rarely needed in modern deployments.
//!
//! # Why chmod is Not Supported
//!
//! **Permissions are handled at the system level**: In modern containerized and
//! orchestrated deployments, file permissions are managed by the infrastructure
//! (Dockerfile USER directives, systemd service configs, etc.) rather than at runtime.
//!
//! **Files already have appropriate permissions**: When agents create files via MCP
//! tools, they're created with sensible default permissions. Changing permissions
//! after creation is rarely necessary.
//!
//! **Simplifies the mental model**: Rather than thinking about permission bits,
//! focus on the content and organization of files. The system handles access control.

use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// chmod override - educates about permission management
#[derive(Parser)]
pub struct ChmodCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for ChmodCommand {
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
                "Error: 'chmod' is not available in this shell.\n\n\
                 In modern deployments, file permissions are managed at the system level.\n\n\
                 For file modifications, use KODEGEN's filesystem tools:\n\n\
                 • fs_edit_block - Edit file contents with exact block replacement\n\
                   - Atomic operations (all-or-nothing writes)\n\
                   - Preserves file permissions automatically\n\
                   - Validates changes before writing\n\n\
                 • fs_write_file - Create or overwrite files\n\
                   - Creates files with appropriate default permissions\n\
                   - Supports both write and append modes\n\
                   - Validates paths before writing\n\n\
                 Common scenarios:\n\n\
                 1. Make script executable:\n\
                    Instead of: chmod +x script.sh\n\
                    Use: fs_write_file with shebang (#!/bin/bash) for shell detection\n\
                    Or: Execute via terminal tool with explicit interpreter (bash script.sh)\n\n\
                 2. Fix configuration file permissions:\n\
                    File permissions are managed by the system/container.\n\
                    If running in a container, adjust Dockerfile USER/RUN directives.\n\n\
                 3. Secure sensitive files:\n\
                    Use fs_write_file to create files - they'll have appropriate defaults.\n\
                    Container/system handles permission isolation.\n\n\
                 Why chmod is not supported:\n\
                 - Permissions are typically set by deployment infrastructure\n\
                 - Files created by MCP tools have sensible defaults\n\
                 - Simplifies the mental model (focus on content, not permission bits)\n\
                 - Modern deployments use containers with built-in isolation\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

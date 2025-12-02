use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// sed override - redirects to kodegen filesystem tools
#[derive(Parser)]
pub struct SedCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    _args: Vec<String>,
}

impl Command for SedCommand {
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
                "Error: 'sed' is not available in this shell.\n\n\
                 Instead, use KODEGEN's filesystem tools for safe, lightning-fast operations:\n\n\
                 • fs_read_file           - Read file contents\n\
                 • fs_read_multiple_files - Read multiple files efficiently\n\
                 • fs_search              - Search file contents with regex\n\
                 • fs_edit_block          - Edit files with exact block replacement\n\n\
                 These tools provide:\n\
                 - Memory-efficient streaming\n\
                 - Atomic file operations\n\
                 - Built-in safety checks\n\
                 - Blazing-fast SIMD operations\n"
            )?;

            Ok(ExecutionResult::new(1))
        }
    }
}

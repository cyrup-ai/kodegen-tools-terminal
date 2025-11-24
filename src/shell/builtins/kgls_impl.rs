//! Shared kgls executor for ls and lsd builtins

use brush_core::commands::ExecutionContext;
use brush_core::{ExecutionResult, Error};
use clap::Parser;
use std::io::Write;

/// Execute kgls with the given arguments
pub async fn execute_kgls(
    args: Vec<String>,
    context: ExecutionContext<'_>,
) -> Result<ExecutionResult, Error> {
    // Prepend "kgls" as argv[0] for clap parsing
    let mut argv = vec!["kgls".to_string()];
    argv.extend(args);

    // Parse into Cli using clap
    let cli = match kgls::Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(e) => {
            writeln!(context.stderr(), "{}", e)?;
            return Ok(ExecutionResult::new(2));
        }
    };

    // Use default config (no config file in terminal context)
    let config = kgls::Config::default();

    // Build flags from cli + config
    let flags = match kgls::Flags::configure_from(&cli, &config) {
        Ok(flags) => flags,
        Err(e) => {
            writeln!(context.stderr(), "{}", e)?;
            return Ok(ExecutionResult::new(2));
        }
    };

    // Create core and run (kgls prints directly to stdout/stderr)
    let core = kgls::Core::new(flags);
    let exit_code = core.run(cli.inputs).await;

    // Convert kgls::ExitCode to u8 (OK=0, MinorIssue=1, MajorIssue=2)
    let exit_code_i32: i32 = exit_code.into();
    Ok(ExecutionResult::new(exit_code_i32 as u8))
}

//! ls builtin - delegates to kgls

use brush_core::builtins::Command;
use brush_core::commands::ExecutionContext;
use brush_core::{ExecutionResult, Error};
use clap::Parser;

#[derive(Parser)]
pub struct LsCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

impl Command for LsCommand {
    type Error = Error;

    #[allow(clippy::manual_async_fn)]
    fn execute(
        &self,
        context: ExecutionContext<'_>,
    ) -> impl std::future::Future<Output = Result<ExecutionResult, Self::Error>>
           + std::marker::Send {
        async move {
            super::kgls_impl::execute_kgls(self.args.clone(), context).await
        }
    }
}

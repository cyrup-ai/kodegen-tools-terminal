//! lsd builtin - delegates to kgls (drop-in lsd replacement)

use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error};
use clap::Parser;

#[derive(Parser)]
pub struct LsdCommand {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

impl Command for LsdCommand {
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

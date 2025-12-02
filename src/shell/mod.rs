pub mod kodegen_shell;
pub use kodegen_shell::KodegenShell;

pub mod interactive;
pub use interactive::{ShellHandle, KodegenInteractiveThread};

mod builtins;

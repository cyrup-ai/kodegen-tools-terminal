//! listpkgs builtin - lists all packages in a workspace
//! 
//! This is a test builtin to verify complex shell operations work correctly.
//! It performs: cd to directory, loop over subdirectories, print each name.
//!
//! Usage: listpkgs [path]
//!   path: Optional path to packages directory (default: ./packages)

use kodegen_bash_shell::builtins::Command;
use kodegen_bash_shell::ExecutionContext;
use kodegen_bash_shell::{ExecutionResult, Error, ExitCode};
use clap::Parser;
use std::io::Write;

#[derive(Parser)]
#[command(name = "listpkgs", about = "List all packages in a workspace")]
pub struct ListPkgsCommand {
    /// Path to the packages directory (default: ./packages)
    #[arg(default_value = "./packages")]
    path: String,
}

impl Command for ListPkgsCommand {
    type Error = Error;

    #[allow(clippy::manual_async_fn)]
    fn execute(
        &self,
        context: ExecutionContext<'_>,
    ) -> impl std::future::Future<Output = Result<ExecutionResult, Self::Error>>
           + std::marker::Send {
        let path = self.path.clone();
        async move {
            // Get stdout for writing
            let mut stdout = context.stdout();
            
            // Resolve the path relative to current working directory
            let packages_path = if path.starts_with('/') {
                std::path::PathBuf::from(&path)
            } else {
                let cwd = context.shell.working_dir();
                cwd.join(&path)
            };
            
            // Check if path exists
            if !packages_path.exists() {
                let _ = writeln!(stdout, "Error: Path does not exist: {}", packages_path.display());
                return Ok(ExecutionResult::new(ExitCode::Custom(1)));
            }
            
            if !packages_path.is_dir() {
                let _ = writeln!(stdout, "Error: Path is not a directory: {}", packages_path.display());
                return Ok(ExecutionResult::new(ExitCode::Custom(1)));
            }
            
            let _ = writeln!(stdout, "=== Listing packages in {} ===", packages_path.display());
            let _ = writeln!(stdout);
            
            // Read directory entries
            let entries = match std::fs::read_dir(&packages_path) {
                Ok(entries) => entries,
                Err(e) => {
                    let _ = writeln!(stdout, "Error reading directory: {}", e);
                    return Ok(ExecutionResult::new(ExitCode::Custom(1)));
                }
            };
            
            // Collect and sort entries
            let mut package_names: Vec<String> = entries
                .filter_map(|entry| entry.ok())
                .filter(|entry| entry.path().is_dir())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| !name.starts_with('.'))  // Skip hidden directories
                .collect();
            
            package_names.sort();
            
            // Print each package with a simulated loop iteration
            let total = package_names.len();
            for (i, name) in package_names.iter().enumerate() {
                let _ = writeln!(stdout, "[{}/{}] Package: {}", i + 1, total, name);
                
                // Check for Cargo.toml to verify it's a Rust package
                let cargo_toml = packages_path.join(&name).join("Cargo.toml");
                if cargo_toml.exists() {
                    let _ = writeln!(stdout, "       └── ✓ Cargo.toml found");
                } else {
                    let _ = writeln!(stdout, "       └── ✗ No Cargo.toml");
                }
            }
            
            let _ = writeln!(stdout);
            let _ = writeln!(stdout, "=== Total: {} packages ===", total);
            
            Ok(ExecutionResult::new(ExitCode::Success))
        }
    }
}

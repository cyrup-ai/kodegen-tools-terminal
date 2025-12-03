use kodegen_bash_shell::{Shell, builtin, openfiles, default_builtins, BuiltinSet};
use std::io;
use std::path::PathBuf;

use super::builtins::{SedCommand, LsCommand, LsdCommand, FindCommand, MvCommand, ChmodCommand, ChownCommand, LnCommand, KillCommand, KillallCommand, PkillCommand};

#[derive(Clone)]
pub struct KodegenShell {
    shell: Shell,
}

impl KodegenShell {
    pub async fn new(working_dir: Option<PathBuf>) -> io::Result<Self> {
        // Get default builtins
        let mut builtins = default_builtins(BuiltinSet::BashMode);

        // Override sed to redirect to kodegen filesystem tools
        builtins.insert("sed".to_string(), builtin::<SedCommand>());

        // Override find, mv to redirect to kodegen filesystem tools
        builtins.insert("find".to_string(), builtin::<FindCommand>());
        builtins.insert("mv".to_string(), builtin::<MvCommand>());

        // Override chmod, chown, ln - educational builtins (no MCP replacements)
        builtins.insert("chmod".to_string(), builtin::<ChmodCommand>());
        builtins.insert("chown".to_string(), builtin::<ChownCommand>());
        builtins.insert("ln".to_string(), builtin::<LnCommand>());

        // Override kill, killall, pkill - redirect to process management tools
        builtins.insert("kill".to_string(), builtin::<KillCommand>());
        builtins.insert("killall".to_string(), builtin::<KillallCommand>());
        builtins.insert("pkill".to_string(), builtin::<PkillCommand>());

        // Override ls and lsd with kgls (blazing-fast ls/lsd replacement)
        builtins.insert("ls".to_string(), builtin::<LsCommand>());
        builtins.insert("lsd".to_string(), builtin::<LsdCommand>());

        let shell = match working_dir {
            Some(dir) => {
                Shell::builder()
                    .interactive(true)
                    .builtins(builtins)
                    .working_dir(dir)
                    .build()
                    .await
                    .map_err(|e| io::Error::other(format!("Failed to create shell: {}", e)))?
            }
            None => {
                Shell::builder()
                    .interactive(true)
                    .builtins(builtins)
                    .build()
                    .await
                    .map_err(|e| io::Error::other(format!("Failed to create shell: {}", e)))?
            }
        };

        Ok(Self { shell })
    }

    /// Create shell with custom stdin/stdout/stderr file descriptors
    pub async fn with_fds(
        stdin: openfiles::OpenFile,
        stdout: openfiles::OpenFile,
        stderr: openfiles::OpenFile,
    ) -> io::Result<Self> {
        use std::collections::HashMap;

        // Set custom FDs for stdin (0), stdout (1) and stderr (2)
        let fds: HashMap<_, _> = [
            (openfiles::OpenFiles::STDIN_FD, stdin),
            (openfiles::OpenFiles::STDOUT_FD, stdout),
            (openfiles::OpenFiles::STDERR_FD, stderr),
        ].into_iter().collect();

        // Get default builtins
        let mut builtins = default_builtins(BuiltinSet::BashMode);

        // Override sed to redirect to kodegen filesystem tools
        builtins.insert("sed".to_string(), builtin::<SedCommand>());

        // Override find, mv to redirect to kodegen filesystem tools
        builtins.insert("find".to_string(), builtin::<FindCommand>());
        builtins.insert("mv".to_string(), builtin::<MvCommand>());

        // Override chmod, chown, ln - educational builtins (no MCP replacements)
        builtins.insert("chmod".to_string(), builtin::<ChmodCommand>());
        builtins.insert("chown".to_string(), builtin::<ChownCommand>());
        builtins.insert("ln".to_string(), builtin::<LnCommand>());

        // Override kill, killall, pkill - redirect to process management tools
        builtins.insert("kill".to_string(), builtin::<KillCommand>());
        builtins.insert("killall".to_string(), builtin::<KillallCommand>());
        builtins.insert("pkill".to_string(), builtin::<PkillCommand>());

        // Override ls and lsd with kgls (blazing-fast ls/lsd replacement)
        builtins.insert("ls".to_string(), builtin::<LsCommand>());
        builtins.insert("lsd".to_string(), builtin::<LsdCommand>());

        let shell = Shell::builder()
            .interactive(true)
            .builtins(builtins)
            .fds(fds)
            .build()
            .await
            .map_err(|e| io::Error::other(format!("Failed to create shell: {}", e)))?;

        Ok(Self { shell })
    }

    /// Get mutable reference to shell for command execution
    pub fn shell_mut(&mut self) -> &mut Shell {
        &mut self.shell
    }

    /// Get reference to shell for state inspection
    pub fn shell(&self) -> &Shell {
        &self.shell
    }
}

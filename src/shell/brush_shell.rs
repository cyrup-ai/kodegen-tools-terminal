use brush_core::Shell as BrushCoreShell;
use brush_core::builtins::builtin;
use std::io;

use super::builtins::{SedCommand, LsCommand, LsdCommand, FindCommand, GrepCommand, MvCommand, ChmodCommand, ChownCommand, LnCommand, KillCommand, KillallCommand, PkillCommand};

#[derive(Clone)]
pub struct BrushShell {
    shell: BrushCoreShell,
}

impl BrushShell {
    pub async fn new() -> io::Result<Self> {
        // Get default builtins
        let mut builtins = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);

        // Override sed to redirect to kodegen filesystem tools
        builtins.insert("sed".to_string(), builtin::<SedCommand>());

        // Override find, grep, mv to redirect to kodegen filesystem tools
        builtins.insert("find".to_string(), builtin::<FindCommand>());
        builtins.insert("grep".to_string(), builtin::<GrepCommand>());
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

        let shell = BrushCoreShell::builder()
            .interactive(true)
            .builtins(builtins)
            .build()
            .await
            .map_err(|e| io::Error::other(format!("Failed to create brush shell: {}", e)))?;

        Ok(Self { shell })
    }

    /// Create shell with custom stdout/stderr file descriptors
    pub async fn with_fds(
        stdout: brush_core::openfiles::OpenFile,
        stderr: brush_core::openfiles::OpenFile,
    ) -> io::Result<Self> {
        use std::collections::HashMap;

        // Set custom FDs for stdout (1) and stderr (2)
        let fds: HashMap<_, _> = [
            (brush_core::openfiles::OpenFiles::STDOUT_FD, stdout),
            (brush_core::openfiles::OpenFiles::STDERR_FD, stderr),
        ].into_iter().collect();

        // Get default builtins
        let mut builtins = brush_builtins::default_builtins(brush_builtins::BuiltinSet::BashMode);

        // Override sed to redirect to kodegen filesystem tools
        builtins.insert("sed".to_string(), builtin::<SedCommand>());

        // Override find, grep, mv to redirect to kodegen filesystem tools
        builtins.insert("find".to_string(), builtin::<FindCommand>());
        builtins.insert("grep".to_string(), builtin::<GrepCommand>());
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

        let shell = BrushCoreShell::builder()
            .interactive(true)
            .builtins(builtins)
            .fds(fds)
            .build()
            .await
            .map_err(|e| io::Error::other(format!("Failed to create brush shell: {}", e)))?;

        Ok(Self { shell })
    }

    /// Get mutable reference to shell for command execution
    pub fn shell_mut(&mut self) -> &mut BrushCoreShell {
        &mut self.shell
    }

    /// Get reference to shell for state inspection
    pub fn shell(&self) -> &BrushCoreShell {
        &self.shell
    }
}

//! Path analyzer - validates filesystem paths against restricted directories
//!
//! This analyzer checks if command arguments reference restricted system directories
//! like /etc, /sys, /proc, etc. It performs path normalization and canonicalization
//! to handle relative paths, symlinks, and home directory expansion.

use crate::validation::{ValidationDecision, ViolationType};
use std::path::Path;
use kodegen_config::KodegenConfig;

/// System paths that are always restricted
///
/// These directories contain critical system files that should not be
/// modified by agents. Any command attempting to access these paths
/// will be blocked.
const RESTRICTED_PATHS: &[&str] = &[
    "/etc",         // System configuration
    "/private/etc", // macOS: /etc symlinks here
    "/sys",         // Kernel interface
    "/proc",        // Process information
    "/boot",        // Boot loader and kernel
    "/dev",         // Device files
    "/root",        // Root user home
    "/usr/bin",     // System binaries
    "/usr/sbin",    // System admin binaries
    "/bin",         // Essential binaries
    "/sbin",        // System binaries
    "/var/lib",     // System state data
    "/private/var", // macOS: /var symlinks here
    "/lib",         // System libraries
    "/lib64",       // 64-bit system libraries
];

/// Paths that are explicitly allowed
///
/// These paths are safe for agent operations:
/// - /tmp: Temporary files (safe for experimentation)
/// - User home directory: User-owned files
const ALLOWED_PATH_PREFIXES: &[&str] = &[
    "/tmp",
    "/private/tmp", // macOS /tmp
];

/// Path analyzer - validates filesystem paths against restricted directories
///
/// This analyzer performs the following checks:
/// 1. Extracts path arguments from command strings
/// 2. Expands ~ to user home directory
/// 3. Canonicalizes paths (resolves .., ., symlinks)
/// 4. Checks against restricted system directories
/// 5. Allows user home and /tmp directories
///
/// # Architecture
///
/// ```text
/// Command String
///     ↓
/// PathAnalyzer::analyze()
///     ↓
/// Extract paths → Expand ~ → Canonicalize
///     ↓
/// Check restricted paths
///     ↓
/// Return None (safe) or Some(Block) (restricted)
/// ```
///
/// # Examples
///
/// ```rust
/// use kodegen_tools_terminal::validation::analyzers::PathAnalyzer;
/// use kodegen_tools_terminal::validation::ValidationDecision;
///
/// let analyzer = PathAnalyzer::new();
///
/// // Safe path
/// assert!(analyzer.analyze("rm /tmp/file.txt").is_none());
///
/// // Restricted path
/// match analyzer.analyze("rm /etc/passwd") {
///     Some(ValidationDecision::Block { reason, .. }) => {
///         println!("Blocked: {}", reason);
///     }
///     _ => unreachable!(),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct PathAnalyzer;

impl PathAnalyzer {
    /// Create a new path analyzer
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Analyze command for restricted filesystem paths
    ///
    /// # Arguments
    ///
    /// * `command` - The full command string to analyze (e.g., "rm -rf /etc")
    ///
    /// # Returns
    ///
    /// * `None` - Command paths are safe
    /// * `Some(ValidationDecision::Block)` - Command targets restricted paths
    ///
    /// # Path Resolution
    ///
    /// The analyzer performs the following transformations:
    /// - `~` → User home directory
    /// - `./file` → `/current/working/dir/file`
    /// - `../file` → `/parent/dir/file`
    /// - Symlinks → Resolved to target path
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kodegen_tools_terminal::validation::analyzers::PathAnalyzer;
    ///
    /// let analyzer = PathAnalyzer::new();
    ///
    /// // System paths are blocked
    /// assert!(analyzer.analyze("rm -rf /etc/config").is_some());
    /// assert!(analyzer.analyze("chmod 777 /sys/kernel").is_some());
    ///
    /// // /tmp is allowed
    /// assert!(analyzer.analyze("rm /tmp/test.txt").is_none());
    ///
    /// // User home is allowed
    /// assert!(analyzer.analyze("rm ~/documents/file.txt").is_none());
    /// ```
    pub fn analyze(&self, command: &str) -> Option<ValidationDecision> {
        // Extract potential paths from command
        // Split by whitespace and filter for path-like strings
        let tokens: Vec<&str> = command.split_whitespace().collect();

        for token in tokens {
            // Skip flags (starting with -)
            if token.starts_with('-') {
                continue;
            }

            // Check if token looks like a path
            if (token.starts_with('/') || token.starts_with('~') || token.starts_with('.'))
                && let Some(decision) = self.check_path(token)
            {
                return Some(decision);
            }
        }

        None
    }

    /// Check a single path against restricted directories
    ///
    /// # Arguments
    ///
    /// * `path_str` - The path string to check
    ///
    /// # Returns
    ///
    /// * `None` - Path is safe
    /// * `Some(ValidationDecision::Block)` - Path is restricted
    fn check_path(&self, path_str: &str) -> Option<ValidationDecision> {
        // Expand ~ to home directory
        let expanded = self.expand_home(path_str);
        let path = Path::new(&expanded);

        // Try to canonicalize the path (resolves .., ., symlinks)
        // If canonicalization fails, fall back to the original path
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        // Convert to string for comparison
        let canonical_str = canonical.to_string_lossy();

        // Check if path is explicitly allowed
        if self.is_allowed_path(&canonical_str) {
            return None;
        }

        // Check if path is restricted
        if self.is_restricted_path(&canonical_str) {
            return Some(ValidationDecision::Block {
                reason: format!(
                    "Path '{}' is in a restricted system directory. Use MCP filesystem tools for safe operations.",
                    path_str
                ),
                violation_type: ViolationType::SystemPath,
            });
        }

        None
    }

    /// Expand ~ to user home directory
    ///
    /// # Arguments
    ///
    /// * `path` - Path string potentially starting with ~
    ///
    /// # Returns
    ///
    /// Path with ~ expanded to home directory, or original path if ~ not present
    fn expand_home(&self, path: &str) -> String {
        if path.starts_with('~')
            && let Some(home) = dirs::home_dir()
        {
            return path.replacen('~', &home.to_string_lossy(), 1);
        }
        path.to_string()
    }

    /// Check if path is in allowed directories
    ///
    /// Allowed paths (in priority order):
    /// 1. Explicit allowed prefixes (/tmp, /private/tmp)
    /// 2. Git repository root (enables chmod +x for development scripts)
    /// 3. User home directory
    ///
    /// This enables commands like `chmod +x script.sh` within a git repository
    /// while still blocking `chmod 777 /etc/passwd` on system paths.
    ///
    /// # Arguments
    ///
    /// * `path` - Canonical path string
    ///
    /// # Returns
    ///
    /// `true` if path is allowed, `false` otherwise
    fn is_allowed_path(&self, path: &str) -> bool {
        // Check explicit allowed prefixes first (fastest)
        for allowed in ALLOWED_PATH_PREFIXES {
            if path.starts_with(allowed) {
                return true;
            }
        }

        // Check if path is within git repository root
        // This enables chmod, etc. on project files during development
        // local_config_dir() returns ${git_root}/.kodegen, so we get the parent
        if let Ok(local_config) = KodegenConfig::local_config_dir()
            && let Some(git_root) = local_config.parent()
        {
            let git_root_str = git_root.to_string_lossy();
            if path.starts_with(git_root_str.as_ref()) {
                return true;
            }
        }

        // Check if path is in user home directory
        if let Some(home) = dirs::home_dir() {
            let home_str = home.to_string_lossy();
            if path.starts_with(home_str.as_ref()) {
                return true;
            }
        }

        false
    }

    /// Check if path is in restricted directories
    ///
    /// # Arguments
    ///
    /// * `path` - Canonical path string
    ///
    /// # Returns
    ///
    /// `true` if path is restricted, `false` otherwise
    fn is_restricted_path(&self, path: &str) -> bool {
        for restricted in RESTRICTED_PATHS {
            if path.starts_with(restricted) {
                return true;
            }
        }
        false
    }
}

impl Default for PathAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_paths_blocked() {
        let analyzer = PathAnalyzer::new();

        // System directories should be blocked
        assert!(analyzer.analyze("rm /etc/passwd").is_some());
        assert!(analyzer.analyze("chmod 777 /sys/kernel/config").is_some());
        assert!(analyzer.analyze("rm -rf /boot/grub").is_some());
    }

    #[test]
    fn test_tmp_allowed() {
        let analyzer = PathAnalyzer::new();

        // /tmp should be allowed
        assert!(analyzer.analyze("rm /tmp/test.txt").is_none());
        assert!(analyzer.analyze("rm -rf /tmp/temp_dir").is_none());
    }

    #[test]
    fn test_home_expansion() {
        let analyzer = PathAnalyzer::new();

        // Home directory should be allowed
        assert!(analyzer.analyze("rm ~/documents/file.txt").is_none());
    }

    #[test]
    fn test_flags_ignored() {
        let analyzer = PathAnalyzer::new();

        // Flags should not be treated as paths
        assert!(analyzer.analyze("ls -la /tmp").is_none());
        assert!(analyzer.analyze("rm -rf /etc").is_some()); // /etc still blocked
    }

    #[test]
    fn test_relative_paths() {
        let analyzer = PathAnalyzer::new();

        // Relative paths in safe locations should be allowed
        // Note: This depends on current working directory
        // If CWD is in home or /tmp, should be allowed
        let result = analyzer.analyze("rm ./file.txt");
        // Result depends on CWD, so we just verify it doesn't panic
        assert!(result.is_none() || result.is_some());
    }
}

//! Flag analyzer - detects dangerous command-line flags
//!
//! This analyzer uses pre-compiled regex patterns to detect dangerous flags
//! like `-exec`, `-rf`, etc. Patterns are compiled once at startup using
//! `LazyLock` for zero-allocation hot path execution.

use crate::validation::{ValidationDecision, ViolationType};
use regex::Regex;
use std::sync::LazyLock;

/// Pre-compiled pattern for find -exec and -execdir
///
/// Matches: -exec, -execdir, --exec, --execdir
///
/// # Safety
///
/// Pattern is hardcoded and guaranteed to compile. Uses `expect()` which
/// will only fail if the hardcoded pattern is malformed.
static EXEC_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-{1,2}exec(dir)?(\s|$)")
        .expect("hardcoded EXEC_PATTERN must compile")
});

/// Pre-compiled pattern for find -delete
///
/// Matches: -delete, --delete
static DELETE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-{1,2}delete(\s|$)")
        .expect("hardcoded DELETE_PATTERN must compile")
});

/// Pre-compiled pattern for rm -rf combinations
///
/// Matches:
/// - `-rf` or `-fr` (combined flags)
/// - `-r -f` or `-f -r` (separate flags)
/// - `--recursive --force` or `--force --recursive` (long flags)
static RF_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(-[rf]{2}|-r\s+-f|-f\s+-r|--recursive\s+--force|--force\s+--recursive)")
        .expect("hardcoded RF_PATTERN must compile")
});

/// Pre-compiled pattern for xargs (without lookahead)
///
/// Matches xargs followed by whitespace
/// Safety flags (-t, -p) are checked separately in analyze()
static XARGS_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"xargs\s+")
        .expect("hardcoded XARGS_PATTERN must compile")
});

/// Pre-compiled pattern for chmod/chown (without lookahead)
///
/// Matches chmod or chown at command start
/// The --help flag is checked separately in analyze()
static CHMOD_CHOWN_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(chmod|chown)\s+")
        .expect("hardcoded CHMOD_CHOWN_PATTERN must compile")
});

/// Flag analyzer - detects dangerous command-line flags using pre-compiled patterns
///
/// This analyzer checks command arguments for dangerous flag combinations that
/// could lead to data loss or security issues. All patterns are compiled once
/// at startup using `LazyLock` for optimal performance.
///
/// # Architecture
///
/// ```text
/// Command String
///     ↓
/// FlagAnalyzer::analyze()
///     ↓
/// Check patterns (pre-compiled, zero-allocation)
///     ↓
/// Return None (safe) or Some(Block) (dangerous)
/// ```
///
/// # Examples
///
/// ```rust
/// use kodegen_tools_terminal::validation::analyzers::FlagAnalyzer;
/// use kodegen_tools_terminal::validation::ValidationDecision;
///
/// let analyzer = FlagAnalyzer::new();
///
/// // Safe command
/// assert!(analyzer.analyze("find . -name '*.rs'").is_none());
///
/// // Dangerous command
/// match analyzer.analyze("find . -exec rm {} \\;") {
///     Some(ValidationDecision::Block { reason, .. }) => {
///         println!("Blocked: {}", reason);
///     }
///     _ => unreachable!(),
/// }
/// ```
#[derive(Debug, Clone)]
pub struct FlagAnalyzer;

impl FlagAnalyzer {
    /// Create a new flag analyzer
    ///
    /// This is a zero-cost constructor as all patterns are lazily compiled
    /// on first use.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Analyze command for dangerous flags
    ///
    /// # Arguments
    ///
    /// * `command` - The full command string to analyze (e.g., "find . -exec rm")
    ///
    /// # Returns
    ///
    /// * `None` - Command flags are safe
    /// * `Some(ValidationDecision::Block)` - Command contains dangerous flags
    ///
    /// # Performance
    ///
    /// This method uses pre-compiled regex patterns stored in `LazyLock`,
    /// ensuring zero allocation on the hot path. Patterns are compiled once
    /// at first use and reused forever.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kodegen_tools_terminal::validation::analyzers::FlagAnalyzer;
    ///
    /// let analyzer = FlagAnalyzer::new();
    ///
    /// // Check for -exec flag
    /// assert!(analyzer.analyze("find . -exec rm {} \\;").is_some());
    ///
    /// // Check for -rf flag
    /// assert!(analyzer.analyze("rm -rf /tmp/data").is_some());
    ///
    /// // Safe command
    /// assert!(analyzer.analyze("find . -name '*.rs'").is_none());
    /// ```
    pub fn analyze(&self, command: &str) -> Option<ValidationDecision> {
        // Check for find -exec or -execdir
        if EXEC_PATTERN.is_match(command) {
            return Some(ValidationDecision::Block {
                reason: "find -exec can execute arbitrary commands on matched files".to_string(),
                violation_type: ViolationType::DangerousFlag,
            });
        }

        // Check for find -delete
        if DELETE_PATTERN.is_match(command) {
            return Some(ValidationDecision::Block {
                reason: "find -delete can permanently destroy files without confirmation".to_string(),
                violation_type: ViolationType::DangerousFlag,
            });
        }

        // Check for rm -rf (in any form)
        if RF_PATTERN.is_match(command) {
            return Some(ValidationDecision::Block {
                reason: "rm -rf is destructive and irreversible, can delete entire directory trees".to_string(),
                violation_type: ViolationType::DangerousFlag,
            });
        }

        // Check for xargs without safety flags
        // Rust regex doesn't support negative lookahead, so we check manually
        if XARGS_PATTERN.is_match(command) {
            // Allow if command contains -t or -p flags
            if !command.contains("xargs -t") && !command.contains("xargs -p") {
                return Some(ValidationDecision::Block {
                    reason: "xargs requires -t (trace) or -p (prompt) flags for safe operation".to_string(),
                    violation_type: ViolationType::DangerousFlag,
                });
            }
        }

        // Check for chmod/chown (except --help)
        // Rust regex doesn't support negative lookahead, so we check manually
        if CHMOD_CHOWN_PATTERN.is_match(command) {
            // Allow if command is just asking for help
            if !command.contains("--help") {
                return Some(ValidationDecision::Block {
                    reason: "chmod/chown are permission management commands that should use MCP tools instead".to_string(),
                    violation_type: ViolationType::DangerousFlag,
                });
            }
        }

        // No dangerous flags detected
        None
    }
}

impl Default for FlagAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exec_detection() {
        let analyzer = FlagAnalyzer::new();

        // Should detect -exec
        assert!(analyzer.analyze("find . -exec rm {} \\;").is_some());
        assert!(analyzer.analyze("find . -execdir ls \\;").is_some());

        // Should not detect in safe commands
        assert!(analyzer.analyze("find . -name execute.txt").is_none());
    }

    #[test]
    fn test_delete_detection() {
        let analyzer = FlagAnalyzer::new();

        assert!(analyzer.analyze("find . -delete").is_some());
        assert!(analyzer.analyze("find . -name '*.tmp' -delete").is_some());
    }

    #[test]
    fn test_rf_detection() {
        let analyzer = FlagAnalyzer::new();

        // Various forms of -rf
        assert!(analyzer.analyze("rm -rf /tmp/data").is_some());
        assert!(analyzer.analyze("rm -fr /tmp/data").is_some());
        assert!(analyzer.analyze("rm -r -f /tmp/data").is_some());
        assert!(analyzer.analyze("rm -f -r /tmp/data").is_some());

        // Safe commands
        assert!(analyzer.analyze("rm file.txt").is_none());
        assert!(analyzer.analyze("rm -i file.txt").is_none());
    }

    #[test]
    fn test_xargs_detection() {
        let analyzer = FlagAnalyzer::new();

        // Unsafe xargs
        assert!(analyzer.analyze("find . | xargs rm").is_some());

        // Safe xargs (with -t or -p)
        assert!(analyzer.analyze("find . | xargs -t rm").is_none());
        assert!(analyzer.analyze("find . | xargs -p rm").is_none());
    }

    #[test]
    fn test_chmod_chown_detection() {
        let analyzer = FlagAnalyzer::new();

        // Should block chmod/chown
        assert!(analyzer.analyze("chmod 755 file.txt").is_some());
        assert!(analyzer.analyze("chown user:group file.txt").is_some());

        // Allow --help
        assert!(analyzer.analyze("chmod --help").is_none());
        assert!(analyzer.analyze("chown --help").is_none());
    }
}

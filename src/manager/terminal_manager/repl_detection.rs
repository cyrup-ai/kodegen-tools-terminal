//! REPL detection utilities
//!
//! This module provides functionality for detecting when a REPL (Read-Eval-Print Loop)
//! is ready for input by recognizing common prompt patterns.

use super::constants::REPL_PROMPTS;

/// Detect if a REPL is ready for input by checking for known prompt patterns
///
/// Checks the last non-empty line of output against known REPL prompts including:
/// - Python (`>>> `, `... `)
/// - R (`>> `, `> `)
/// - Shell (`$ `, `# `)
/// - Node.js (`node> `, `> `)
/// - Ruby IRB (`irb> `, `irb(main):`)
/// - Haskell GHCi (`λ> `, `ghci> `)
/// - Julia (`julia> `)
/// - Database shells (mysql, postgres, sqlite)
/// - IPython/Jupyter (`In [N]: `)
pub fn detect_repl_ready(output: &str) -> bool {
    // Get last non-empty line
    let last_line = output
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");

    // Check for exact prompt matches
    if REPL_PROMPTS
        .iter()
        .any(|prompt| last_line.ends_with(prompt))
    {
        return true;
    }

    // Special case for IPython/Jupyter
    if last_line.starts_with("In [") && last_line.contains("]: ") {
        return true;
    }

    false
}

//! Command validation rules and patterns
//!
//! This module defines the structures used to specify validation rules for commands,
//! including dangerous flag patterns and restricted paths.

use std::borrow::Cow;

use super::decision::{ValidationDecision, ViolationType};
use super::ParsedCommand;

/// A validation rule for a specific command
///
/// Rules define whether a command is allowed by default and under what conditions
/// it should be blocked (dangerous flags, restricted paths, or custom validation).
///
/// # Examples
///
/// ```
/// use kodegen_tools_terminal::validation::{CommandRule, BlockPattern, ViolationType};
/// use std::borrow::Cow;
///
/// // Simple rule: always block sudo
/// let sudo_rule = CommandRule {
///     command: "sudo",
///     default_allow: false,
///     block_patterns: vec![],
///     restricted_paths: vec![],
///     custom_validator: None,
/// };
///
/// // Context-aware rule: allow rm except with -rf flag
/// let rm_rule = CommandRule {
///     command: "rm",
///     default_allow: true,
///     block_patterns: vec![
///         BlockPattern {
///             pattern: Cow::Borrowed(r"-rf|-fr"),
///             violation: ViolationType::DangerousFlag,
///             reason: "rm -rf is destructive and irreversible",
///         },
///     ],
///     restricted_paths: vec!["/etc", "/sys"],
///     custom_validator: None,
/// };
/// ```
#[derive(Debug)]
pub struct CommandRule {
    /// Command name (lowercase)
    ///
    /// This should be the base command name without path or arguments.
    /// Example: "rm", "find", "sudo"
    pub command: &'static str,

    /// Allow command by default?
    ///
    /// - `true`: Command is allowed unless a block pattern or restricted path matches
    /// - `false`: Command is always blocked (AlwaysBlocked violation)
    pub default_allow: bool,

    /// Patterns that block the command
    ///
    /// These are regex patterns or literal strings that match dangerous flags or
    /// flag combinations. If any pattern matches, the command is blocked.
    pub block_patterns: Vec<BlockPattern>,

    /// Paths that are restricted
    ///
    /// If the command targets any of these paths (or subdirectories), it is blocked
    /// with a SystemPath violation. Paths should be absolute (e.g., "/etc", "/sys").
    pub restricted_paths: Vec<&'static str>,

    /// Custom validation function
    ///
    /// If provided, this function is called after pattern and path checks.
    /// It can implement complex validation logic that can't be expressed as
    /// simple patterns.
    ///
    /// The function receives the parsed command and returns a ValidationDecision.
    /// Return `Allow` to proceed, or `Block` to reject the command.
    pub custom_validator: Option<fn(&ParsedCommand) -> ValidationDecision>,
}

impl CommandRule {
    /// Create a simple rule that always blocks a command
    ///
    /// # Examples
    ///
    /// ```
    /// use kodegen_tools_terminal::validation::CommandRule;
    ///
    /// let rule = CommandRule::always_blocked("sudo");
    /// assert!(!rule.default_allow);
    /// ```
    pub fn always_blocked(command: &'static str) -> Self {
        Self {
            command,
            default_allow: false,
            block_patterns: vec![],
            restricted_paths: vec![],
            custom_validator: None,
        }
    }

    /// Create a rule that allows a command by default with no restrictions
    ///
    /// This is useful as a starting point for rules that will be customized
    /// with the builder pattern.
    ///
    /// # Examples
    ///
    /// ```
    /// use kodegen_tools_terminal::validation::CommandRule;
    ///
    /// let rule = CommandRule::new("ls");
    /// assert!(rule.default_allow);
    /// assert!(rule.block_patterns.is_empty());
    /// ```
    pub fn new(command: &'static str) -> Self {
        Self {
            command,
            default_allow: true,
            block_patterns: vec![],
            restricted_paths: vec![],
            custom_validator: None,
        }
    }

    /// Start building a rule with the builder pattern
    ///
    /// # Examples
    ///
    /// ```
    /// use kodegen_tools_terminal::validation::CommandRule;
    ///
    /// let rule = CommandRule::builder("find")
    ///     .default_allow(true)
    ///     .build();
    /// ```
    pub fn builder(command: &'static str) -> CommandRuleBuilder {
        CommandRuleBuilder {
            command,
            default_allow: true,
            block_patterns: vec![],
            restricted_paths: vec![],
            custom_validator: None,
        }
    }
}

/// Builder for constructing CommandRule instances
///
/// This provides a fluent API for building complex validation rules.
///
/// # Examples
///
/// ```
/// use kodegen_tools_terminal::validation::{CommandRule, BlockPattern, ViolationType};
/// use std::borrow::Cow;
///
/// let rule = CommandRule::builder("find")
///     .default_allow(true)
///     .block_pattern(
///         Cow::Borrowed(r"-exec"),
///         ViolationType::DangerousFlag,
///         "find -exec can execute arbitrary commands"
///     )
///     .restricted_path("/etc")
///     .restricted_path("/sys")
///     .build();
/// ```
pub struct CommandRuleBuilder {
    command: &'static str,
    default_allow: bool,
    block_patterns: Vec<BlockPattern>,
    restricted_paths: Vec<&'static str>,
    custom_validator: Option<fn(&ParsedCommand) -> ValidationDecision>,
}

impl CommandRuleBuilder {
    /// Set whether the command is allowed by default
    pub fn default_allow(mut self, allow: bool) -> Self {
        self.default_allow = allow;
        self
    }

    /// Add a block pattern to the rule
    pub fn block_pattern(
        mut self,
        pattern: Cow<'static, str>,
        violation: ViolationType,
        reason: &'static str,
    ) -> Self {
        self.block_patterns.push(BlockPattern {
            pattern,
            violation,
            reason,
        });
        self
    }

    /// Add a restricted path to the rule
    pub fn restricted_path(mut self, path: &'static str) -> Self {
        self.restricted_paths.push(path);
        self
    }

    /// Set a custom validator function
    pub fn custom_validator(mut self, validator: fn(&ParsedCommand) -> ValidationDecision) -> Self {
        self.custom_validator = Some(validator);
        self
    }

    /// Build the final CommandRule
    pub fn build(self) -> CommandRule {
        CommandRule {
            command: self.command,
            default_allow: self.default_allow,
            block_patterns: self.block_patterns,
            restricted_paths: self.restricted_paths,
            custom_validator: self.custom_validator,
        }
    }
}

/// A pattern that matches dangerous command flags
///
/// When a command's arguments match this pattern, the command is blocked
/// with the specified violation type and reason.
///
/// # Pattern Syntax
///
/// Patterns are Rust regex patterns (see the `regex` crate documentation).
/// Common patterns:
/// - Literal: `"-rf"` matches exactly "-rf"
/// - Alternation: `"-rf|-fr"` matches either "-rf" or "-fr"
/// - Negative lookahead: `"^(?!--safe)"` matches unless "--safe" is present
///
/// # Examples
///
/// ```
/// use kodegen_tools_terminal::validation::{BlockPattern, ViolationType};
/// use std::borrow::Cow;
///
/// // Block rm -rf
/// let pattern = BlockPattern {
///     pattern: Cow::Borrowed(r"-rf|-fr"),
///     violation: ViolationType::DangerousFlag,
///     reason: "rm -rf is destructive and irreversible",
/// };
/// ```
#[derive(Debug)]
pub struct BlockPattern {
    /// Regex or literal flag pattern
    ///
    /// This is a `Cow<'static, str>` to allow both static string literals
    /// (zero-cost) and runtime-generated patterns (owned strings).
    pub pattern: Cow<'static, str>,

    /// Violation type if this pattern matches
    ///
    /// This categorizes the security issue for logging and error messages.
    pub violation: ViolationType,

    /// Human-readable reason for blocking
    ///
    /// This should explain why the pattern is dangerous and what the user
    /// should use instead (typically an MCP tool).
    pub reason: &'static str,
}

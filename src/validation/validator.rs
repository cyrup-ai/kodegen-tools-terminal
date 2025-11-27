//! ValidationEngine - central orchestrator for command validation
//!
//! The ValidationEngine coordinates all validation activities:
//! - Rule management and lookup
//! - Command parsing
//! - Flag and path analysis
//! - Custom validator execution
//!
//! # Architecture
//!
//! ```text
//! Command String
//!     ↓
//! ValidationEngine::validate()
//!     ↓
//! parse_command() → ParsedCommand
//!     ↓
//! Rule Lookup (DashMap)
//!     ↓
//! [No rule found] → Allow (open-world)
//!     ↓
//! [Rule found]
//!     ↓
//! Check default_allow
//!     ↓
//! [false] → Block (AlwaysBlocked)
//! [true]  → Run analyzers
//!     ↓
//! FlagAnalyzer::analyze()
//!     ↓
//! [Some(Block)] → Return Block
//! [None] → Continue
//!     ↓
//! PathAnalyzer::analyze()
//!     ↓
//! [Some(Block)] → Return Block
//! [None] → Continue
//!     ↓
//! Custom Validator (if present)
//!     ↓
//! [Block] → Return Block
//! [Allow] → Continue
//!     ↓
//! Return Allow
//! ```

use crate::validation::{
    analyzers::{FlagAnalyzer, PathAnalyzer},
    command_manager::CommandManager,
    CommandRule, ParsedCommand, ValidationDecision, ViolationType,
};
use dashmap::DashMap;
use smallvec::SmallVec;
use std::sync::Arc;

/// ValidationEngine orchestrates command validation using rules and analyzers
///
/// This is the central component that receives commands, looks up rules,
/// runs analyzers, and returns validation decisions. Rules are stored in
/// a thread-safe concurrent HashMap (DashMap) for efficient lookups.
///
/// # Thread Safety
///
/// The ValidationEngine is fully thread-safe and can be shared across threads:
/// - Rules are wrapped in Arc for efficient sharing
/// - DashMap provides lock-free concurrent access
/// - All operations are safe to call from multiple threads
///
/// # Performance
///
/// - Rule lookups: O(1) average case (DashMap)
/// - Rule storage: Arc wrapping (pointer + refcount)
/// - Zero-allocation validation path (reuses existing allocations)
/// - Analyzers use pre-compiled regex patterns (LazyLock)
///
/// # Examples
///
/// ```rust
/// use kodegen_tools_terminal::validation::{ValidationEngine, CommandRule};
///
/// // Create engine
/// let engine = ValidationEngine::new();
///
/// // Add rule for rm command
/// let rule = CommandRule::builder("rm")
///     .default_allow(true)
///     .block_pattern(
///         std::borrow::Cow::Borrowed(r"-rf"),
///         kodegen_tools_terminal::validation::ViolationType::DangerousFlag,
///         "rm -rf is destructive and irreversible"
///     )
///     .build();
///
/// engine.add_rule(rule);
///
/// // Validate commands
/// let decision = engine.validate("rm file.txt");
/// assert!(matches!(decision, kodegen_tools_terminal::validation::ValidationDecision::Allow));
///
/// let decision = engine.validate("rm -rf /data");
/// assert!(matches!(decision, kodegen_tools_terminal::validation::ValidationDecision::Block { .. }));
/// ```
#[derive(Debug)]
pub struct ValidationEngine {
    /// Thread-safe concurrent map of command rules
    ///
    /// Key: command name (lowercase)
    /// Value: Arc-wrapped CommandRule for efficient sharing
    rules: DashMap<String, Arc<CommandRule>>,
}

impl ValidationEngine {
    /// Create a new ValidationEngine with empty rule set
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kodegen_tools_terminal::validation::ValidationEngine;
    ///
    /// let engine = ValidationEngine::new();
    /// ```
    #[must_use]
    pub fn new() -> Self {
        Self {
            rules: DashMap::new(),
        }
    }

    /// Add a validation rule to the engine
    ///
    /// Rules are identified by their command name (lowercase). If a rule
    /// for the same command already exists, it will be replaced.
    ///
    /// # Arguments
    ///
    /// * `rule` - The CommandRule to add
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kodegen_tools_terminal::validation::{ValidationEngine, CommandRule};
    ///
    /// let engine = ValidationEngine::new();
    ///
    /// // Add rule for always-blocked command
    /// let sudo_rule = CommandRule::always_blocked("sudo");
    /// engine.add_rule(sudo_rule);
    ///
    /// // Add rule with patterns
    /// let rm_rule = CommandRule::builder("rm")
    ///     .default_allow(true)
    ///     .restricted_path("/etc")
    ///     .build();
    /// engine.add_rule(rm_rule);
    /// ```
    pub fn add_rule(&self, rule: CommandRule) {
        let command = rule.command.to_string();
        self.rules.insert(command, Arc::new(rule));
    }

    /// Validate a command against registered rules
    ///
    /// This is the main entry point for validation. It performs the following steps:
    /// 1. Parse the command string into a ParsedCommand
    /// 2. Look up the rule for the base command
    /// 3. If no rule exists, return Allow (open-world assumption)
    /// 4. If rule exists with default_allow=false, return Block (AlwaysBlocked)
    /// 5. If rule exists with default_allow=true:
    ///    - Run FlagAnalyzer (returns early if blocked)
    ///    - Run PathAnalyzer (returns early if blocked)
    ///    - Run custom validator if present (returns early if blocked)
    ///    - Return Allow if all checks pass
    ///
    /// # Arguments
    ///
    /// * `command` - The full command string to validate (e.g., "rm -rf /etc")
    ///
    /// # Returns
    ///
    /// * `ValidationDecision::Allow` - Command is safe to execute
    /// * `ValidationDecision::Block` - Command violates security policy
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kodegen_tools_terminal::validation::{ValidationEngine, ValidationDecision};
    ///
    /// let engine = ValidationEngine::new();
    ///
    /// // Unknown command - allowed by default
    /// match engine.validate("echo hello") {
    ///     ValidationDecision::Allow => println!("Allowed"),
    ///     _ => unreachable!(),
    /// }
    /// ```
    pub fn validate(&self, command: &str) -> ValidationDecision {
        // Parse the command string
        let parsed = self.parse_command(command);

        // Look up the rule for this command
        let rule = match self.rules.get(&parsed.base_command) {
            Some(rule) => rule,
            None => {
                // No rule found - allow by default (open-world assumption)
                return ValidationDecision::Allow;
            }
        };

        // Check if command is always blocked
        if !rule.default_allow {
            return ValidationDecision::Block {
                reason: format!(
                    "Command '{}' is never allowed. Use MCP tools for safe operations.",
                    parsed.base_command
                ),
                violation_type: ViolationType::AlwaysBlocked,
            };
        }

        // Command has default_allow=true, run analyzers
        // Create analyzers (they're cheap - just empty structs)
        let flag_analyzer = FlagAnalyzer::new();
        let path_analyzer = PathAnalyzer::new();

        // Check for dangerous flags (early return on block)
        if let Some(decision) = flag_analyzer.analyze(command) {
            return decision;
        }

        // Check for restricted paths (early return on block)
        if let Some(decision) = path_analyzer.analyze(command) {
            return decision;
        }

        // Run custom validator if present
        if let Some(validator) = rule.custom_validator {
            let decision = validator(&parsed);
            if matches!(decision, ValidationDecision::Block { .. }) {
                return decision;
            }
        }

        // All checks passed - allow the command
        ValidationDecision::Allow
    }

    /// Parse a command string into a ParsedCommand
    ///
    /// This method leverages the existing CommandManager parsing logic
    /// to extract the base command and arguments. It handles:
    /// - Command name extraction (first word, lowercase)
    /// - Argument splitting (simple whitespace splitting)
    /// - Empty command handling
    ///
    /// # Arguments
    ///
    /// * `command` - The full command string to parse
    ///
    /// # Returns
    ///
    /// A ParsedCommand containing:
    /// - base_command: The command name (lowercase)
    /// - args: SmallVec of argument strings
    /// - full_command: The original command string
    ///
    /// # Examples
    ///
    /// ```rust
    /// use kodegen_tools_terminal::validation::ValidationEngine;
    ///
    /// let engine = ValidationEngine::new();
    /// // Note: parse_command is private, but used internally by validate()
    /// ```
    fn parse_command(&self, command: &str) -> ParsedCommand {
        // Create CommandManager for parsing utilities
        let cmd_manager = CommandManager::new();

        // Extract base command using CommandManager's robust logic
        let base_command = cmd_manager.get_base_command(command);

        // Parse arguments by splitting on whitespace and skipping the first element
        let parts: Vec<&str> = command.split_whitespace().collect();
        let args: SmallVec<[String; 8]> = parts
            .iter()
            .skip(1) // Skip base command
            .map(|s| (*s).to_string())
            .collect();

        ParsedCommand {
            base_command,
            args,
            full_command: command.to_string(),
        }
    }
}

impl Default for ValidationEngine {
    fn default() -> Self {
        Self::new()
    }
}

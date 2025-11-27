//! Command validation system with context-aware security rules
//!
//! This module provides a comprehensive command validation framework for terminal
//! command execution. It implements a context-aware security model that blocks
//! dangerous operations while allowing safe commands through.
//!
//! # Architecture
//!
//! The validation system follows a multi-layer architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Terminal::execute_command()              │
//! │                           ↓                                 │
//! │                ValidationEngine::validate()                 │
//! └─────────────────────────────────────────────────────────────┘
//!                              ↓
//!              ┌───────────────┴───────────────┐
//!              ↓                               ↓
//!    ┌──────────────────┐            ┌─────────────────┐
//!    │ CommandManager   │            │ DashMap<Rules>  │
//!    │ parse_command()  │            │ rule lookup     │
//!    └──────────────────┘            └─────────────────┘
//!              ↓                               ↓
//!       ParsedCommand              ┌───────────┴──────────┐
//!              │                   ↓                      ↓
//!              │         [No rule] → Allow    [Rule found] → Check
//!              │                                            ↓
//!              └────────────────────┬───────────────────────┘
//!                                   ↓
//!                    ┌──────────────┴──────────────┐
//!                    ↓                             ↓
//!           default_allow=false          default_allow=true
//!                    ↓                             ↓
//!           Block (AlwaysBlocked)         Run Analyzers
//!                                                  ↓
//!                                  ┌───────────────┴───────────────┐
//!                                  ↓                               ↓
//!                         FlagAnalyzer::analyze()      PathAnalyzer::analyze()
//!                         (dangerous flags)             (restricted paths)
//!                                  ↓                               ↓
//!                         [Block?] → Return         [Block?] → Return
//!                                  ↓                               ↓
//!                         [Allow] → Continue        [Allow] → Continue
//!                                                                  ↓
//!                                                    Custom Validator (optional)
//!                                                                  ↓
//!                                                    [Block] → Return Block
//!                                                    [Allow] → Return Allow
//! ```
//!
//! # Core Components
//!
//! ## [`ValidationEngine`]
//! Central orchestrator that coordinates all validation activities. Stores rules in
//! a thread-safe concurrent hashmap (`DashMap`) for efficient lookups.
//!
//! ## [`CommandRule`]
//! Defines validation rules for specific commands. Each rule specifies:
//! - Whether the command is allowed by default
//! - Dangerous flag patterns to block
//! - Restricted file paths
//! - Optional custom validation logic
//!
//! ## [`ParsedCommand`]
//! Represents a parsed command with its base command name, arguments, and full text.
//! Uses `SmallVec<[String; 8]>` for efficient argument storage (stack-allocated for ≤8 args).
//!
//! ## Analyzers
//! - [`FlagAnalyzer`]: Detects dangerous command-line flags using pre-compiled regex patterns
//! - [`PathAnalyzer`]: Validates file paths against restricted and allowed path lists
//!
//! ## [`ValidationDecision`]
//! The result of validation: either `Allow` or `Block` with reason and violation type.
//!
//! # Usage
//!
//! ```rust
//! use kodegen_tools_terminal::validation::{ValidationEngine, register_default_rules};
//!
//! // Create engine and register default security rules
//! let engine = ValidationEngine::new();
//! register_default_rules(&engine);
//!
//! // Validate commands
//! let decision = engine.validate("ls -la");
//! assert!(matches!(decision, kodegen_tools_terminal::validation::ValidationDecision::Allow));
//!
//! let decision = engine.validate("rm -rf /");
//! assert!(matches!(decision, kodegen_tools_terminal::validation::ValidationDecision::Block { .. }));
//! ```
//!
//! # Rule Categories
//!
//! Default rules (registered via [`register_default_rules`]) cover five categories:
//!
//! ## 1. Always Blocked Commands
//! Commands that are never allowed due to security risks:
//! - `sudo`, `su`, `doas` - Privilege escalation
//! - `reboot`, `shutdown`, `halt`, `poweroff` - System control
//!
//! ## 2. Destructive Commands
//! Commands that modify or delete data, with pattern-based restrictions:
//! - `rm`, `rmdir` - File deletion
//! - `dd` - Raw disk operations
//! - `shred`, `wipe` - Secure deletion
//! - `mkfs.*`, `fdisk`, `parted` - Disk formatting
//!
//! ## 3. Permission Commands
//! Commands that modify access controls:
//! - `chmod`, `chown`, `chgrp` - File permissions
//! - `chattr`, `setfacl` - Extended attributes
//!
//! ## 4. System Modification Commands
//! Commands that alter system configuration:
//! - `iptables`, `ufw`, `firewall-cmd` - Firewall rules
//! - `systemctl`, `service` - Service management
//! - `mount`, `umount` - Filesystem mounting
//!
//! ## 5. Package Management Commands
//! Commands that install or modify software:
//! - `apt`, `yum`, `dnf`, `pacman` - Package managers
//! - `npm`, `pip`, `gem`, `cargo` - Language-specific installers
//!
//! All default rules are hardcoded in `rules/defaults.rs` for performance and simplicity.
//!
//! # Programmatic Customization
//!
//! Users can add custom rules via the Rust API:
//!
//! ```rust
//! use kodegen_tools_terminal::validation::{ValidationEngine, CommandRule, ViolationType};
//! use std::borrow::Cow;
//!
//! let engine = ValidationEngine::new();
//!
//! // Add custom rule with builder pattern
//! let rule = CommandRule::builder("mycmd")
//!     .default_allow(true)
//!     .block_pattern(
//!         Cow::Borrowed(r"--dangerous-flag"),
//!         ViolationType::DangerousFlag,
//!         "This flag is dangerous"
//!     )
//!     .restricted_path("/etc")
//!     .build();
//!
//! engine.add_rule(rule);
//!
//! // Add always-blocked command
//! engine.add_rule(CommandRule::always_blocked("dangerous-cmd"));
//! ```
//!
//! # Educational Builtins
//!
//! Some common Unix commands are intercepted before validation to provide educational
//! feedback about using MCP tools instead:
//!
//! - `find` → Use `fs_search` tool for fast pattern matching
//! - `grep` → Use `fs_search` tool with content search
//! - `mv` → Use `fs_move_file` tool for safe file operations
//! - `chmod`, `chown` → Not needed (MCP tools handle permissions)
//! - `ln` → Use `fs_move_file` or `fs_write_file` instead
//! - `kill`, `killall`, `pkill` → Use `process_kill` tool
//!
//! These intercepts happen in the shell layer before validation, guiding users
//! toward the safer, more powerful MCP tool alternatives.
//!
//! # Performance Characteristics
//!
//! - **Rule lookup**: O(1) average case via `DashMap` concurrent hashmap
//! - **Rule storage**: Arc-wrapped for efficient sharing across threads
//! - **Zero-allocation validation**: Reuses existing allocations where possible
//! - **Pre-compiled patterns**: Regex patterns compiled once at startup (`LazyLock`)
//! - **Thread-safe**: All operations safe for concurrent access
//! - **Open-world assumption**: Unknown commands allowed by default (no lookup overhead)

use smallvec::SmallVec;

// Module declarations
pub mod analyzers;
pub mod command_manager;
pub mod command_rule;
pub mod decision;
pub mod rules;
pub mod validator;

// Re-export core types for convenience
pub use analyzers::{FlagAnalyzer, PathAnalyzer};
pub use command_manager::CommandManager;
pub use command_rule::{BlockPattern, CommandRule};
pub use decision::{ValidationDecision, ViolationType};
pub use rules::register_default_rules;
pub use validator::ValidationEngine;

/// Represents a parsed shell command with its components
///
/// This struct is used to pass command information to validators and analyzers.
/// The `args` field uses `SmallVec` with an inline capacity of 8, which means
/// commands with 8 or fewer arguments avoid heap allocation entirely.
///
/// # Optimization
///
/// Most shell commands have fewer than 8 arguments, so `SmallVec<[String; 8]>`
/// provides optimal performance for the common case while gracefully handling
/// commands with many arguments (falls back to heap allocation).
///
/// # Examples
///
/// ```
/// use kodegen_tools_terminal::validation::ParsedCommand;
/// use smallvec::SmallVec;
///
/// // Simple command with few arguments (stack-allocated)
/// let ls_cmd = ParsedCommand {
///     base_command: "ls".to_string(),
///     args: SmallVec::from_vec(vec!["-la".to_string()]),
///     full_command: "ls -la".to_string(),
/// };
///
/// // Command with many arguments (heap-allocated)
/// let find_cmd = ParsedCommand {
///     base_command: "find".to_string(),
///     args: SmallVec::from_vec(vec![
///         ".".to_string(),
///         "-name".to_string(),
///         "*.rs".to_string(),
///         // ... more arguments
///     ]),
///     full_command: "find . -name *.rs".to_string(),
/// };
/// ```
#[derive(Debug, Clone)]
pub struct ParsedCommand {
    /// The base command name (lowercase, without path)
    ///
    /// Examples: "rm", "find", "sudo"
    pub base_command: String,

    /// Command arguments
    ///
    /// Uses SmallVec with inline capacity of 8 for optimal performance.
    /// Most commands have < 8 args, so this avoids heap allocation.
    pub args: SmallVec<[String; 8]>,

    /// The full original command string
    ///
    /// This is preserved for logging and error messages.
    pub full_command: String,
}

impl ParsedCommand {
    /// Create a new ParsedCommand
    ///
    /// # Examples
    ///
    /// ```
    /// use kodegen_tools_terminal::validation::ParsedCommand;
    /// use smallvec::SmallVec;
    ///
    /// let cmd = ParsedCommand::new(
    ///     "ls".to_string(),
    ///     SmallVec::from_vec(vec!["-la".to_string()]),
    ///     "ls -la".to_string(),
    /// );
    /// ```
    pub fn new(base_command: String, args: SmallVec<[String; 8]>, full_command: String) -> Self {
        Self {
            base_command,
            args,
            full_command,
        }
    }

    /// Check if the command has any arguments
    pub fn has_args(&self) -> bool {
        !self.args.is_empty()
    }

    /// Get the number of arguments
    pub fn arg_count(&self) -> usize {
        self.args.len()
    }

    /// Check if a specific argument is present
    ///
    /// # Examples
    ///
    /// ```
    /// use kodegen_tools_terminal::validation::ParsedCommand;
    /// use smallvec::SmallVec;
    ///
    /// let cmd = ParsedCommand::new(
    ///     "rm".to_string(),
    ///     SmallVec::from_vec(vec!["-rf".to_string(), "/tmp/test".to_string()]),
    ///     "rm -rf /tmp/test".to_string(),
    /// );
    ///
    /// assert!(cmd.has_arg("-rf"));
    /// assert!(!cmd.has_arg("-v"));
    /// ```
    pub fn has_arg(&self, arg: &str) -> bool {
        self.args.iter().any(|a| a == arg)
    }

    /// Get all arguments as a slice
    pub fn args_slice(&self) -> &[String] {
        &self.args
    }
}

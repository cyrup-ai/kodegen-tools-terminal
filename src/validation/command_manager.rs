//! Command parsing utilities
//!
//! This module provides utility functions for parsing and extracting information
//! from shell command strings. It handles complex cases like compound commands,
//! environment variable prefixes, and full paths.
//!
//! **Note**: For command validation, use `ValidationEngine` (in `validator.rs`).
//! This module only provides parsing utilities.
//!
//! # Utilities:
//! - `get_base_command()` - Extract command name from full command string
//! - `extract_commands()` - Split compound commands (&&, ||, ;, |)
//! - `extract_base_command()` - Handle environment variable prefixes

use log::warn;
use regex::Regex;
use std::collections::HashSet;

// Compile regex once at startup (not on every command validation)
static ENV_VAR_REGEX: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
    match Regex::new(r"\w+=\S+\s*") {
        Ok(regex) => regex,
        Err(e) => {
            // This pattern is hardcoded and tested, so this should never happen
            // If it does, it indicates a compile-time programming error
            panic!(
                "FATAL: Hardcoded regex pattern r\"\\w+=\\S+\\s*\" failed to compile: {e}\n\
                 This is a programming bug that should be fixed in the source code."
            );
        }
    }
});

/// Command manager for parsing commands
#[derive(Clone, Copy)]
pub struct CommandManager;

impl CommandManager {
    /// Create a new command manager instance
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Extract the base command (first word, lowercase, trimmed) from a command string
    /// Handles full paths by extracting just the executable name
    #[must_use]
    pub fn get_base_command(&self, command: &str) -> String {
        let first_word = command.split_whitespace().next().unwrap_or("").trim();

        // Extract basename from path (handles /bin/rm, /usr/bin/sudo, ../../bin/chmod, etc.)
        let basename = if first_word.contains('/') || first_word.contains('\\') {
            // Use std::path::Path for cross-platform path handling
            std::path::Path::new(first_word)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(first_word)
        } else {
            first_word
        };

        basename.to_lowercase()
    }

    /// Extract all commands from a command string, handling quotes, escaping, and separators
    /// Returns empty Vec on parsing error (safer than permissive fallback)
    #[must_use]
    pub fn extract_commands(&self, command_string: &str) -> Vec<String> {
        match self.extract_commands_internal(command_string) {
            Ok(commands) => commands,
            Err(e) => {
                // Log the error with full command for debugging
                log::error!(
                    "Error extracting commands from '{command_string}': {e}. Treating as potentially malicious."
                );

                // SAFER: Return empty Vec to trigger validation failure
                // validate_command() will check if empty and use get_base_command() as fallback
                // This prevents bypasses via deliberately broken parsing
                Vec::new()
            }
        }
    }

    /// Internal implementation for extracting commands
    /// Handles quotes, escape sequences, command separators, and nested structures
    fn extract_commands_internal(&self, command_string: &str) -> Result<Vec<String>, String> {
        let command_string = command_string.trim();
        if command_string.is_empty() {
            return Ok(Vec::new());
        }

        // Define command separators
        let separators = [";", "&&", "||", "|", "&"];
        let mut commands: Vec<String> = Vec::new();

        // State for parsing
        let mut in_quote = false;
        let mut quote_char = '\0';
        let mut current_cmd = String::new();
        let mut escaped = false;

        let chars: Vec<char> = command_string.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            let ch = chars[i];

            // Handle escape characters
            if ch == '\\' && !escaped {
                escaped = true;
                current_cmd.push(ch);
                i += 1;
                continue;
            }

            // If this character is escaped, just add it
            if escaped {
                escaped = false;
                current_cmd.push(ch);
                i += 1;
                continue;
            }

            // Handle quotes (both single and double)
            if (ch == '"' || ch == '\'') && !in_quote {
                in_quote = true;
                quote_char = ch;
                current_cmd.push(ch);
                i += 1;
                continue;
            } else if ch == quote_char && in_quote {
                in_quote = false;
                quote_char = '\0';
                current_cmd.push(ch);
                i += 1;
                continue;
            }

            // If we're inside quotes, just add the character
            if in_quote {
                current_cmd.push(ch);
                i += 1;
                continue;
            }

            // Handle subshells - if we see an opening parenthesis
            if ch == '(' {
                let subshell_end = Self::find_matching_paren(&chars, i)?;
                if subshell_end > i + 1 {
                    let subshell_content: String =
                        chars[(i + 1)..(subshell_end - 1)].iter().collect();
                    // Recursively extract commands from the subshell
                    let sub_commands = self.extract_commands_internal(&subshell_content)?;
                    commands.extend(sub_commands);
                    i = subshell_end;
                    continue;
                }
            }

            // Check for separators
            let mut is_separator = false;
            for separator in &separators {
                if Self::starts_with_at(&chars, i, separator) {
                    // We found a separator - extract the command before it
                    if !current_cmd.trim().is_empty()
                        && let Some(base_command) = self.extract_base_command(current_cmd.trim())
                    {
                        commands.push(base_command);
                    }

                    // Move past the separator
                    i += separator.len();
                    current_cmd.clear();
                    is_separator = true;
                    break;
                }
            }

            if !is_separator {
                current_cmd.push(ch);
                i += 1;
            }
        }

        // Don't forget to add the last command
        if !current_cmd.trim().is_empty()
            && let Some(base_command) = self.extract_base_command(current_cmd.trim())
        {
            commands.push(base_command);
        }

        // Remove duplicates and return
        let unique_commands: Vec<String> = commands
            .into_iter()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        Ok(unique_commands)
    }

    /// Find the matching closing parenthesis for an opening parenthesis at position start
    fn find_matching_paren(chars: &[char], start: usize) -> Result<usize, String> {
        if start >= chars.len() || chars[start] != '(' {
            return Err("Invalid starting position for parenthesis matching".to_string());
        }

        let mut open_parens = 1;
        let mut j = start + 1;

        while j < chars.len() && open_parens > 0 {
            if chars[j] == '(' {
                open_parens += 1;
            } else if chars[j] == ')' {
                open_parens -= 1;
            }
            j += 1;
        }

        if open_parens == 0 {
            Ok(j)
        } else {
            Err("Unmatched parentheses".to_string())
        }
    }

    /// Check if the chars slice starts with the given string at the given position
    fn starts_with_at(chars: &[char], pos: usize, s: &str) -> bool {
        let s_chars: Vec<char> = s.chars().collect();
        if pos + s_chars.len() > chars.len() {
            return false;
        }

        for (i, &expected_char) in s_chars.iter().enumerate() {
            if chars[pos + i] != expected_char {
                return false;
            }
        }
        true
    }

    /// Extract the actual command name from a command string
    /// Removes environment variables and returns the base command
    #[must_use]
    pub fn extract_base_command(&self, command_str: &str) -> Option<String> {
        if let Ok(cmd) = Self::extract_base_command_internal(command_str) {
            cmd
        } else {
            warn!("Error extracting base command from: {command_str}");
            None
        }
    }

    /// Internal implementation for extracting base command
    fn extract_base_command_internal(command_str: &str) -> Result<Option<String>, String> {
        // Remove environment variables using pre-compiled regex
        let without_env_vars = ENV_VAR_REGEX.replace_all(command_str, "");
        let trimmed = without_env_vars.trim();

        // If nothing remains after removing env vars, return None
        if trimmed.is_empty() {
            return Ok(None);
        }

        // Get the first token (the command)
        let tokens: Vec<&str> = trimmed.split_whitespace().collect();
        if tokens.is_empty() {
            return Ok(None);
        }

        let first_token = tokens[0];

        // Check if it starts with special characters that might indicate it's not a regular command
        if first_token.starts_with('(') || first_token.starts_with('$') {
            return Ok(None);
        }

        // Extract basename from path (handles /bin/rm, /usr/bin/sudo, ../../bin/chmod, etc.)
        let basename = if first_token.contains('/') || first_token.contains('\\') {
            std::path::Path::new(first_token)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(first_token)
        } else {
            first_token
        };

        Ok(Some(basename.to_lowercase()))
    }

}

impl Default for CommandManager {
    fn default() -> Self {
        Self::new()
    }
}

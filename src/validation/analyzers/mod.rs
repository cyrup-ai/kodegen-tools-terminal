//! Command analyzers for validation
//!
//! This module provides analyzers that check command strings for dangerous
//! patterns and restricted filesystem paths.
//!
//! # Analyzers
//!
//! - [`FlagAnalyzer`] - Detects dangerous command-line flags
//! - [`PathAnalyzer`] - Validates filesystem paths against restricted directories
//!
//! # Usage
//!
//! ```rust
//! use kodegen_tools_terminal::validation::analyzers::{FlagAnalyzer, PathAnalyzer};
//!
//! let flag_analyzer = FlagAnalyzer::new();
//! let path_analyzer = PathAnalyzer::new();
//!
//! // Check command for dangerous flags
//! if let Some(decision) = flag_analyzer.analyze("find . -exec rm {} \\;") {
//!     println!("Blocked by flag analyzer");
//! }
//!
//! // Check command for restricted paths
//! if let Some(decision) = path_analyzer.analyze("rm /etc/passwd") {
//!     println!("Blocked by path analyzer");
//! }
//! ```

mod flag_analyzer;
mod path_analyzer;

pub use flag_analyzer::FlagAnalyzer;
pub use path_analyzer::PathAnalyzer;

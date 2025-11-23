//! Constants for terminal session management
//!
//! This module defines constants used throughout the terminal manager:
//! - Cleanup retention configuration
//! - Output buffer limits
//! - Session limits
//! - REPL prompt patterns

/// Cleanup interval for background task (seconds)
pub const CLEANUP_INTERVAL_SECS: u64 = 60; // Check every minute

/// Retention time for active sessions (seconds)
pub const ACTIVE_SESSION_RETENTION_SECS: u64 = 5 * 60; // 5 minutes for active sessions

/// Retention time for completed sessions (seconds)
pub const COMPLETED_SESSION_RETENTION_SECS: u64 = 30; // 30 seconds for completed sessions

/// Maximum lines per session in output buffer
pub const MAX_OUTPUT_BUFFER_LINES: usize = 10_000;

/// Maximum concurrent sessions allowed
pub const MAX_SESSIONS: usize = 100;

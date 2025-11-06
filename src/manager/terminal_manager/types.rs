//! Data types for terminal session management
//!
//! This module defines all the data structures used by the terminal manager:
//! - Session information and metadata
//! - API response types
//! - Metrics and statistics

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

// ============================================================================
// METRICS
// ============================================================================

/// Metrics for monitoring terminal session health and performance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalMetrics {
    /// Total sessions created since server start
    pub total_sessions_created: u64,
    /// Currently active sessions
    pub active_sessions: usize,
    /// Sessions in `completed_sessions` map
    pub completed_sessions: usize,
    /// Average session duration in seconds
    pub average_session_duration_secs: f64,
    /// Maximum concurrent sessions reached
    pub max_concurrent_sessions: usize,
    /// Total commands executed
    pub total_commands_executed: u64,
}

// ============================================================================
// SESSION TYPES
// ============================================================================

/// Terminal session information for internal tracking (active sessions)
#[derive(Clone)]
pub struct TerminalSessionInfo {
    pub pid: u32,
    pub command: String,

    // NEW: Direct terminal reference encapsulates all state
    pub terminal: Arc<RwLock<crate::pty::Terminal>>,

    // KEEP: Activity tracking for auto-cleanup
    pub last_read_time: Arc<RwLock<Instant>>,

    // KEEP: Existing fields unchanged
    pub is_blocked: bool,
    pub ready_for_input: bool,
    pub start_time: DateTime<Utc>,
}

/// Active terminal session information for external API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActiveTerminalSession {
    pub pid: u32,
    pub is_blocked: bool,
    /// Runtime in milliseconds
    pub runtime: u64,
}

/// Response for `get_output` (paginated terminal output)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalOutputResponse {
    /// Process ID
    pub pid: u32,

    /// Lines returned in this page
    pub lines: Vec<String>,

    /// Total lines currently buffered
    pub total_lines: usize,

    /// Number of lines in this response
    pub lines_returned: usize,

    /// Process has finished executing
    pub is_complete: bool,

    /// Exit code (if process completed)
    pub exit_code: Option<i32>,

    /// More output may be available (check again)
    pub has_more: bool,

    /// Indicates if buffer reached size limit (early output may be lost)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_truncated: Option<bool>,
}

/// Completed terminal session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedTerminalSession {
    pub pid: u32,
    pub output: String,
    pub exit_code: Option<i32>,
    pub start_time: std::time::SystemTime,
    pub end_time: std::time::SystemTime,
}

/// Result of terminal command execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalCommandResult {
    pub pid: u32,
    pub output: String,
    pub is_blocked: bool,
    pub ready_for_input: bool,
}

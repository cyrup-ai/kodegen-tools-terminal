//! Event types for the three-component architecture

use std::path::PathBuf;

// ============================================================================
// BRUSHEXECUTOR EVENTS (Thread 1 → Thread 2)
// ============================================================================

/// Events produced by BrushExecutor
/// Consumed by VteProcessor
#[derive(Debug, Clone)]
pub enum ShellOutput {
    /// Raw bytes from command execution (stdout or stderr combined)
    Bytes {
        request_id: rmcp::model::RequestId,
        data: Vec<u8>,
    },

    /// Command execution completed
    /// This is the "is_final" indicator
    /// Includes current working directory after command execution
    ExecComplete {
        request_id: rmcp::model::RequestId,
        exit_code: u8,
        cwd: PathBuf,
    },
}

// ============================================================================
// VTEPROCESSOR EVENTS (Thread 2 → Thread 3/API)
// ============================================================================

/// Events produced by VteProcessor
/// Consumed by TerminalManager subscribers
#[derive(Debug, Clone)]
pub enum TerminalBuffer {
    /// Terminal grid updated with new content
    /// Contains the FULL terminal buffer (all visible lines)
    /// Includes current working directory from most recent command
    Updated {
        request_id: rmcp::model::RequestId,
        lines: Vec<String>,
        cursor_line: usize,
        cursor_col: usize,
        cwd: PathBuf,
        exit_code: i32,
        is_final: bool,  // true when command execution complete
    },

    /// Title escape sequence processed (no request_id - async VTE event)
    TitleChanged {
        title: String,
    },
}

// ============================================================================
// API → BRUSHEXECUTOR COMMANDS
// ============================================================================

/// Commands sent from API to BrushExecutor
#[derive(Debug)]
pub enum ExecuteCommand {
    /// Execute a shell command
    Run {
        request_id: rmcp::model::RequestId,
        command: String,
    },
}

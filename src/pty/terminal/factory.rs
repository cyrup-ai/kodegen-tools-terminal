use super::{
    builder::TerminalBuilder,
    types::Terminal,
};

impl Terminal {
    /// Create a convenient builder for terminal creation
    #[must_use]
    pub fn builder() -> TerminalBuilder {
        TerminalBuilder::new()
    }

}

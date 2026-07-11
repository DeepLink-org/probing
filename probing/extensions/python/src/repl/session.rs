//! WebSocket REPL session facade (server owns transport; python ext owns REPL state).

use super::{PythonRepl, Repl};

/// One interactive REPL session for `/ws` (text in → JSON/text out).
pub struct ReplSession {
    repl: PythonRepl,
}

impl Default for ReplSession {
    fn default() -> Self {
        Self::new()
    }
}

impl ReplSession {
    pub fn new() -> Self {
        Self {
            repl: PythonRepl::default(),
        }
    }

    /// Feed one WebSocket text frame; returns the REPL response string.
    pub fn handle_text(&mut self, text: String) -> String {
        self.repl.feed(text).unwrap_or_else(|| "{}".to_string())
    }

    /// Evaluate one code snippet (HTTP eval API — single shot, no line buffering).
    pub fn eval_code(&mut self, code: &str) -> String {
        self.repl.process(code).unwrap_or_default()
    }
}

//! WebSocket REPL session facade (server owns transport; python ext owns REPL state).

use log;

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
        match self.repl.feed(text) {
            Some(response) => response,
            None => {
                log::error!("REPL feed returned no response");
                r#"{"error":"repl feed failed"}"#.to_string()
            }
        }
    }

    /// Evaluate one code snippet (HTTP eval API — single shot, no line buffering).
    pub fn eval_code(&mut self, code: &str) -> String {
        match self.repl.process(code) {
            Some(response) => response,
            None => {
                log::error!("REPL eval returned no response");
                r#"{"error":"repl eval failed"}"#.to_string()
            }
        }
    }
}

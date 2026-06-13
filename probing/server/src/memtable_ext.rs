//! Mmap memtable ↔ SQL integration.
//!
//! The implementation moved to `probing_core::core::memtable_sql` so that both
//! the server and language extensions can expose mmap memtables to SQL through
//! the same code path (logical chunk ordering, generation re-validation, and
//! zero-copy mmap reads). This module re-exports it for backward compatibility.

pub use probing_core::core::memtable_sql::*;

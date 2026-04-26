//! MEMH: self-describing, self-contained pure key-value hash table.
//!
//! ## Quick start
//!
//! ```rust,no_run
//! use probing_memtable::memh::{init_buf, MemhView, MemhWriter};
//! use probing_memtable::Value;
//!
//! let mut buf = vec![0u8; 8 * 1024];
//! init_buf(&mut buf, 64, 4096, 0).unwrap();
//!
//! let mut writer = MemhWriter::new(&mut buf).unwrap();
//! writer.insert("hello", &Value::I64(42)).unwrap();
//! drop(writer);
//!
//! let view = MemhView::new(&buf).unwrap();
//! let val = view.get("hello");
//! println!("{:?}", val);
//! ```

pub mod codec;
pub mod layout;
pub mod table;

pub use codec::TypedValue;
pub use layout::{MAGIC_MEMH, VERSION_MEMH};
pub use table::{
    init_buf, validate_memh, view_from_buf, writer_from_buf, InsertError, InsertResult,
    MemhInitError, MemhValidateError, MemhView, MemhWriter, SharedMemhWriter,
};

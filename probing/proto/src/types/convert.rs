//! Unified type conversion system for Ele type
//!
//! This module provides a centralized and extensible type conversion system
//! to replace scattered conversion logic throughout the codebase.

use crate::types::error::ProtoError;
use crate::types::Ele;

/// Trait for converting Ele to various types
///
/// This trait provides a unified interface for converting Ele values
/// to different target types with proper error handling.
pub trait FromEle: Sized {
    /// Convert an Ele value to this type
    ///
    /// # Errors
    /// Returns `ProtoError::WrongElementType` if the conversion is not possible
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError>;
}

/// Trait for converting values to Ele
///
/// This trait extends the standard `Into<Ele>` trait with additional
/// conversion capabilities.
pub trait ToEle {
    /// Convert this value to Ele
    fn to_ele(self) -> Ele;
}

// Implement ToEle for types that already have Into<Ele>
impl<T: Into<Ele>> ToEle for T {
    fn to_ele(self) -> Ele {
        self.into()
    }
}

// Implement FromEle for common types
impl FromEle for String {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::Text(s) => Ok(s.clone()),
            Ele::Url(s) => Ok(s.clone()),
            Ele::BOOL(b) => Ok(if *b { "True".to_string() } else { "False".to_string() }),
            Ele::I32(i) => Ok(i.to_string()),
            Ele::I64(i) => Ok(i.to_string()),
            Ele::F32(f) => Ok(f.to_string()),
            Ele::F64(f) => Ok(f.to_string()),
            Ele::DataTime(t) => Ok(t.to_string()),
            Ele::Nil => Ok("nil".to_string()),
        }
    }
}

impl FromEle for i32 {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::I32(x) => Ok(*x),
            Ele::I64(x) => {
                if *x >= i32::MIN as i64 && *x <= i32::MAX as i64 {
                    Ok(*x as i32)
                } else {
                    Err(ProtoError::WrongElementType)
                }
            }
            _ => Err(ProtoError::WrongElementType),
        }
    }
}

impl FromEle for i64 {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::I32(x) => Ok(*x as i64),
            Ele::I64(x) => Ok(*x),
            _ => Err(ProtoError::WrongElementType),
        }
    }
}

impl FromEle for f32 {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::F32(x) => Ok(*x),
            Ele::F64(x) => Ok(*x as f32),
            _ => Err(ProtoError::WrongElementType),
        }
    }
}

impl FromEle for f64 {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::F32(x) => Ok(*x as f64),
            Ele::F64(x) => Ok(*x),
            _ => Err(ProtoError::WrongElementType),
        }
    }
}

impl FromEle for bool {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::BOOL(b) => Ok(*b),
            _ => Err(ProtoError::WrongElementType),
        }
    }
}

impl FromEle for Option<String> {
    fn from_ele(ele: &Ele) -> Result<Self, ProtoError> {
        match ele {
            Ele::Nil => Ok(None),
            _ => String::from_ele(ele).map(Some),
        }
    }
}

/// Extension trait for Ele to provide convenient conversion methods
pub trait EleExt {
    /// Convert Ele to String, handling all types
    fn to_string_lossy(&self) -> String;

    /// Try to convert Ele to a specific type
    fn try_into<T: FromEle>(&self) -> Result<T, ProtoError>;

    /// Convert Ele to String if it's a text type, otherwise return None
    fn as_str(&self) -> Option<&str>;

    /// Convert Ele to i64 if it's an integer type
    fn as_i64(&self) -> Option<i64>;

    /// Convert Ele to f64 if it's a float type
    fn as_f64(&self) -> Option<f64>;

    /// Convert Ele to bool if it's a boolean type
    fn as_bool(&self) -> Option<bool>;
}

impl EleExt for Ele {
    fn to_string_lossy(&self) -> String {
        String::from_ele(self).unwrap_or_else(|_| "unknown".to_string())
    }

    fn try_into<T: FromEle>(&self) -> Result<T, ProtoError> {
        T::from_ele(self)
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Ele::Text(s) | Ele::Url(s) => Some(s.as_str()),
            _ => None,
        }
    }

    fn as_i64(&self) -> Option<i64> {
        match self {
            Ele::I32(x) => Some(*x as i64),
            Ele::I64(x) => Some(*x),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Ele::F32(x) => Some(*x as f64),
            Ele::F64(x) => Some(*x),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Ele::BOOL(b) => Some(*b),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_ele_string() {
        assert_eq!(String::from_ele(&Ele::Text("hello".to_string())).unwrap(), "hello");
        assert_eq!(String::from_ele(&Ele::BOOL(true)).unwrap(), "True");
        assert_eq!(String::from_ele(&Ele::I32(42)).unwrap(), "42");
        assert_eq!(String::from_ele(&Ele::Nil).unwrap(), "nil");
    }

    #[test]
    fn test_from_ele_i32() {
        assert_eq!(i32::from_ele(&Ele::I32(42)).unwrap(), 42);
        assert_eq!(i32::from_ele(&Ele::I64(42)).unwrap(), 42);
        assert!(i32::from_ele(&Ele::I64(i64::MAX)).is_err());
    }

    #[test]
    fn test_ele_ext() {
        let ele = Ele::Text("test".to_string());
        assert_eq!(ele.as_str(), Some("test"));
        assert_eq!(ele.to_string_lossy(), "test");

        let ele = Ele::I64(42);
        assert_eq!(ele.as_i64(), Some(42));
        assert_eq!(ele.to_string_lossy(), "42");
    }
}


//! Arrow array to Seq conversion utilities
//!
//! This module provides unified conversion functions from Arrow arrays to Seq,
//! replacing hardcoded type conversion logic throughout the codebase.

use arrow::array::ArrayRef;
use arrow::array::*;
use probing_proto::prelude::Seq;

/// Convert Arrow ArrayRef to Seq
///
/// This function provides a unified way to convert Arrow arrays to Seq,
/// replacing hardcoded type conversion logic throughout the codebase.
pub fn arrow_array_to_seq(array: &ArrayRef) -> Seq {
    if let Some(arr) = array.as_any().downcast_ref::<Int32Array>() {
        Seq::SeqI32(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<Int64Array>() {
        Seq::SeqI64(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<Float32Array>() {
        Seq::SeqF32(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<Float64Array>() {
        Seq::SeqF64(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<StringArray>() {
        Seq::SeqText((0..array.len()).map(|i| arr.value(i).to_string()).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<BooleanArray>() {
        Seq::SeqBOOL((0..array.len()).map(|i| arr.value(i)).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<TimestampMicrosecondArray>() {
        // Convert timestamp to i64 (microseconds)
        Seq::SeqI64(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<TimestampNanosecondArray>() {
        // Convert nanosecond timestamp to i64 (nanoseconds)
        Seq::SeqI64(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<TimestampMillisecondArray>() {
        // Convert millisecond timestamp to i64 (milliseconds)
        Seq::SeqI64(arr.values().to_vec())
    } else if let Some(arr) = array.as_any().downcast_ref::<TimestampSecondArray>() {
        // Convert second timestamp to i64 (seconds)
        Seq::SeqI64(arr.values().to_vec())
    } else {
        // Fallback: return Nil for unsupported types
        Seq::Nil
    }
}

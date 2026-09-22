//! Arrow array to Seq conversion utilities
//!
//! This module provides unified conversion functions from Arrow arrays to Seq,
//! replacing hardcoded type conversion logic throughout the codebase.

use arrow::array::ArrayRef;
use arrow::array::*;
use arrow::datatypes::DataType;
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
    } else if let Some(arr) = array.as_any().downcast_ref::<UInt8Array>() {
        // The table store widens `bool` to `u8`, so this is how boolean columns
        // such as `rl.sample.reward_pass` come back out of SQL.
        Seq::SeqI32(arr.values().iter().map(|value| i32::from(*value)).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<Int8Array>() {
        Seq::SeqI32(arr.values().iter().map(|value| i32::from(*value)).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<UInt16Array>() {
        Seq::SeqI32(arr.values().iter().map(|value| i32::from(*value)).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<Int16Array>() {
        Seq::SeqI32(arr.values().iter().map(|value| i32::from(*value)).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<UInt32Array>() {
        Seq::SeqI64(arr.values().iter().map(|value| i64::from(*value)).collect())
    } else if let Some(arr) = array.as_any().downcast_ref::<UInt64Array>() {
        Seq::SeqI64(arr.values().iter().map(|value| *value as i64).collect())
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

/// Empty column matching an Arrow type (for zero-row query results).
pub fn empty_seq_for_data_type(data_type: &DataType) -> Seq {
    match data_type {
        DataType::Int32 => Seq::SeqI32(vec![]),
        DataType::Int64 => Seq::SeqI64(vec![]),
        DataType::Float32 => Seq::SeqF32(vec![]),
        DataType::Float64 => Seq::SeqF64(vec![]),
        DataType::Utf8 | DataType::LargeUtf8 => Seq::SeqText(vec![]),
        DataType::Boolean => Seq::SeqBOOL(vec![]),
        DataType::UInt8 | DataType::Int8 | DataType::UInt16 | DataType::Int16 => {
            Seq::SeqI32(vec![])
        }
        DataType::UInt32 | DataType::UInt64 => Seq::SeqI64(vec![]),
        DataType::Timestamp(_, _) => Seq::SeqI64(vec![]),
        _ => Seq::Nil,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn narrow_integers_survive_the_round_trip() {
        // Every one of these used to fall through to `Seq::Nil`, silently
        // dropping the whole column from query results.
        let u8s: ArrayRef = Arc::new(UInt8Array::from(vec![0_u8, 1, 255]));
        assert_eq!(arrow_array_to_seq(&u8s), Seq::SeqI32(vec![0, 1, 255]));

        let i8s: ArrayRef = Arc::new(Int8Array::from(vec![-1_i8, 0, 127]));
        assert_eq!(arrow_array_to_seq(&i8s), Seq::SeqI32(vec![-1, 0, 127]));

        let u16s: ArrayRef = Arc::new(UInt16Array::from(vec![7_u16, 65535]));
        assert_eq!(arrow_array_to_seq(&u16s), Seq::SeqI32(vec![7, 65535]));

        let u32s: ArrayRef = Arc::new(UInt32Array::from(vec![4_294_967_295_u32]));
        assert_eq!(arrow_array_to_seq(&u32s), Seq::SeqI64(vec![4_294_967_295]));
    }

    #[test]
    fn a_widened_bool_column_reads_back_as_truthy_integers() {
        // `bool` is stored as `u8`, so a boolean column arrives as UInt8.
        let flags: ArrayRef = Arc::new(UInt8Array::from(vec![1_u8, 0, 1]));
        assert_eq!(arrow_array_to_seq(&flags), Seq::SeqI32(vec![1, 0, 1]));
    }

    #[test]
    fn empty_results_keep_their_column_type() {
        assert_eq!(
            empty_seq_for_data_type(&DataType::UInt8),
            Seq::SeqI32(vec![])
        );
        assert_eq!(
            empty_seq_for_data_type(&DataType::UInt64),
            Seq::SeqI64(vec![])
        );
        assert_eq!(
            empty_seq_for_data_type(&DataType::Boolean),
            Seq::SeqBOOL(vec![])
        );
    }
}

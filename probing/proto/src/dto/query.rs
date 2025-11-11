//! Query DTO (Data Transfer Object) definitions for the query API
//!
//! These structures provide a stable interface for external clients
//! while allowing internal implementations to evolve independently.

use serde::{Deserialize, Serialize};

/// Query request DTO for external API clients
#[derive(Debug, Deserialize, Serialize)]
pub struct QueryRequestDto {
    /// SQL expression or query command
    pub expr: String,

    /// Optional query options
    pub opts: Option<QueryOptionsDto>,
}

/// Query options DTO
#[derive(Debug, Deserialize, Serialize)]
pub struct QueryOptionsDto {
    /// Maximum number of rows to return
    pub limit: Option<usize>,
}

impl QueryRequestDto {
    /// Create a new query request DTO
    pub fn new(expr: String) -> Self {
        Self { expr, opts: None }
    }

    /// Create a new query request DTO with options
    pub fn with_options(expr: String, limit: Option<usize>) -> Self {
        Self {
            expr,
            opts: Some(QueryOptionsDto { limit }),
        }
    }
}

/// Query response DTO for external API clients
#[derive(Debug, Serialize)]
pub struct QueryResponseDto {
    /// Query result data
    pub payload: QueryDataDto,

    /// Response timestamp in microseconds since epoch
    pub timestamp: u64,

    /// Indicates if the query was successful
    pub success: bool,

    /// Optional message for error cases
    pub message: Option<String>,
}

/// Query data variants for response DTO
#[derive(Debug, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum QueryDataDto {
    /// Empty result (e.g., for SET commands)
    Nil,

    /// Error response
    Error {
        /// Error code
        code: String,

        /// Error message
        message: String,
    },

    /// Data frame result
    DataFrame(super::dataframe::DataFrame),

    /// Time series result
    TimeSeries(super::time_series::TimeSeries),
}

impl QueryResponseDto {
    /// Create a successful response with payload
    pub fn success(payload: QueryDataDto) -> Self {
        Self {
            payload,
            timestamp: Self::now(),
            success: true,
            message: None,
        }
    }

    /// Create an error response
    pub fn error(code: String, message: String) -> Self {
        Self {
            payload: QueryDataDto::Error {
                code,
                message: message.clone(),
            },
            timestamp: Self::now(),
            success: false,
            message: Some(message),
        }
    }

    /// Create a nil response
    pub fn nil() -> Self {
        Self {
            payload: QueryDataDto::Nil,
            timestamp: Self::now(),
            success: true,
            message: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn now() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64
    }

    #[cfg(target_arch = "wasm32")]
    fn now() -> u64 {
        0
    }
}

/// Convert internal Query to DTO
impl From<crate::protocol::query::Query> for QueryRequestDto {
    fn from(query: crate::protocol::query::Query) -> Self {
        Self {
            expr: query.expr,
            opts: query.opts.map(|opts| QueryOptionsDto { limit: opts.limit }),
        }
    }
}

/// Convert DTO to internal Query
impl From<QueryRequestDto> for crate::protocol::query::Query {
    fn from(dto: QueryRequestDto) -> Self {
        Self {
            expr: dto.expr,
            opts: dto
                .opts
                .map(|opts| crate::protocol::query::Options { limit: opts.limit }),
        }
    }
}

/// Convert internal Ele to DTO Ele
fn convert_ele(ele: crate::types::basic::Ele) -> super::basic::Ele {
    match ele {
        crate::types::basic::Ele::Nil => super::basic::Ele::Nil,
        crate::types::basic::Ele::BOOL(x) => super::basic::Ele::BOOL(x),
        crate::types::basic::Ele::I32(x) => super::basic::Ele::I32(x),
        crate::types::basic::Ele::I64(x) => super::basic::Ele::I64(x),
        crate::types::basic::Ele::F32(x) => super::basic::Ele::F32(x),
        crate::types::basic::Ele::F64(x) => super::basic::Ele::F64(x),
        crate::types::basic::Ele::Text(x) => super::basic::Ele::Text(x),
        crate::types::basic::Ele::Url(x) => super::basic::Ele::Url(x),
        crate::types::basic::Ele::DataTime(x) => super::basic::Ele::DataTime(x),
    }
}

/// Convert internal Data to DTO
impl From<crate::protocol::query::Data> for QueryDataDto {
    fn from(data: crate::protocol::query::Data) -> Self {
        match data {
            crate::protocol::query::Data::Nil => QueryDataDto::Nil,
            crate::protocol::query::Data::Error(error) => QueryDataDto::Error {
                code: format!("{:?}", error.code),
                message: error.message,
            },
            crate::protocol::query::Data::DataFrame(df) => {
                let cols = df
                    .cols
                    .into_iter()
                    .map(|seq| match seq {
                        crate::types::Seq::Nil => super::basic::Seq::Nil,
                        crate::types::Seq::SeqBOOL(vec) => super::basic::Seq::SeqBOOL(vec),
                        crate::types::Seq::SeqI32(vec) => super::basic::Seq::SeqI32(vec),
                        crate::types::Seq::SeqI64(vec) => super::basic::Seq::SeqI64(vec),
                        crate::types::Seq::SeqF32(vec) => super::basic::Seq::SeqF32(vec),
                        crate::types::Seq::SeqF64(vec) => super::basic::Seq::SeqF64(vec),
                        crate::types::Seq::SeqText(vec) => super::basic::Seq::SeqText(vec),
                        crate::types::Seq::SeqDateTime(vec) => super::basic::Seq::SeqDateTime(vec),
                    })
                    .collect();

                QueryDataDto::DataFrame(super::dataframe::DataFrame {
                    names: df.names,
                    cols,
                    size: df.size,
                })
            }
            crate::protocol::query::Data::TimeSeries(ts) => {
                let timestamp = ts.timestamp.iter().map(convert_ele).collect();
                let cols = ts
                    .cols
                    .into_iter()
                    .map(|series| series.iter().map(convert_ele).collect())
                    .collect();

                QueryDataDto::TimeSeries(super::time_series::TimeSeries {
                    names: ts.names,
                    timestamp,
                    cols,
                })
            }
        }
    }
}

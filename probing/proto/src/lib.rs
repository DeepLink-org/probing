pub mod dto;
pub mod protocol;
pub mod types;

pub mod prelude {
    // --- Protocol Structures ---
    pub use crate::protocol::cluster::{Cluster, Node};
    pub use crate::protocol::message::Message;
    pub use crate::protocol::process::{CallFrame, Process};

    pub use crate::protocol::query::{Data as QueryDataFormat, Options as QueryOptions, Query};
    pub use crate::protocol::query::{ErrorCode, QueryError};
    pub use crate::protocol::version::ProtocolVersion;

    // --- Core Data Types ---
    pub use crate::types::DataFrame;
    pub use crate::types::Ele;
    pub use crate::types::Seq;
    pub use crate::types::TimeSeries;
    pub use crate::types::Value;
    pub use crate::types::{DiscardStrategy, Series};

    // --- Type Conversion ---
    pub use crate::types::{EleExt, FromEle, ToEle};

    // --- Error Handling ---
    pub use crate::types::ProtoError;

    // --- DTO Structures ---
    pub use crate::dto::query::{QueryDataDto, QueryOptionsDto, QueryRequestDto, QueryResponseDto};
}

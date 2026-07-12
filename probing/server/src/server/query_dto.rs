//! Query DTO handler functions
//!
//! This module contains all the functions related to handling query DTOs,
//! separated from the main server module for better organization.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use probing_proto::protocol::message::Message;
use probing_proto::protocol::query::{Data as ProtoData, Query as ProtoQuery};
use serde_json;

use crate::server::error::ApiError;

/// HTTP handler wrapper for query endpoint with DTO interface
/// This provides a stable external API while keeping the internal implementation unchanged
#[axum::debug_handler]
pub async fn query_dto(
    axum::extract::Json(request_dto): axum::extract::Json<
        probing_proto::dto::query::QueryRequestDto,
    >,
) -> impl IntoResponse {
    handle_query_dto(request_dto).await
}

/// Handle query DTO processing and convert to internal format
async fn handle_query_dto(
    request_dto: probing_proto::dto::query::QueryRequestDto,
) -> impl IntoResponse {
    if let Some(msg) = crate::engine_lifecycle::engine_not_ready_message() {
        return convert_engine_error_to_dto(ApiError::service_unavailable(msg)).await;
    }

    // Convert DTO to internal Query structure
    let query: ProtoQuery = request_dto.into();

    // Wrap in Message for internal processing
    let message = Message::new(query);

    // Serialize to JSON string for existing engine interface
    match serde_json::to_string(&message) {
        Ok(json_request) => process_engine_query(json_request).await,
        Err(e) => (
            StatusCode::BAD_REQUEST,
            format!("Failed to serialize request: {}", e),
        )
            .into_response(),
    }
}

/// Process the engine query and convert response to DTO format
async fn process_engine_query(json_request: String) -> axum::response::Response {
    match crate::engine::query(json_request).await {
        Ok(envelope) => convert_engine_response_to_dto(envelope.body, envelope.partial).await,
        Err(api_error) => convert_engine_error_to_dto(api_error).await,
    }
}

/// Convert engine response to DTO format
async fn convert_engine_response_to_dto(
    response_json: String,
    partial: bool,
) -> axum::response::Response {
    // Parse the response to convert to DTO format
    match serde_json::from_str::<Message<ProtoData>>(&response_json) {
        Ok(message_response) => {
            if let ProtoData::Error(err) = &message_response.payload {
                let status = engine_error_status(err);
                let error_response = probing_proto::dto::query::QueryResponseDto::error(
                    api_error_code(status).to_string(),
                    format!("{:?}: {}", err.code, err.message),
                );
                return match serde_json::to_string(&error_response) {
                    Ok(error_json) => (status, error_json).into_response(),
                    Err(e) => (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("Failed to serialize error response: {}", e),
                    )
                        .into_response(),
                };
            }

            let meta_partial = message_response
                .meta
                .as_ref()
                .and_then(|m| m.get("fanout"))
                .and_then(|f| f.get("partial"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let partial = partial || meta_partial;

            let response_dto = probing_proto::dto::query::QueryResponseDto::success(
                message_response.payload.into(),
            );

            match serde_json::to_string(&response_dto) {
                Ok(dto_response_json) => {
                    let status = if partial {
                        StatusCode::SERVICE_UNAVAILABLE
                    } else {
                        StatusCode::OK
                    };
                    (status, dto_response_json).into_response()
                }
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Failed to serialize DTO response: {}", e),
                )
                    .into_response(),
            }
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to parse engine response: {}", e),
        )
            .into_response(),
    }
}

fn engine_error_status(err: &probing_proto::protocol::query::QueryError) -> StatusCode {
    use probing_proto::protocol::query::ErrorCode;
    match err.code {
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::ParseError | ErrorCode::PermissionDenied => StatusCode::BAD_REQUEST,
        ErrorCode::TimeoutError | ErrorCode::ResourceExhausted => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::ExecutionError | ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// Convert engine error to DTO error response
async fn convert_engine_error_to_dto(api_error: ApiError) -> axum::response::Response {
    let status = api_error.status();
    let error_response = probing_proto::dto::query::QueryResponseDto::error(
        api_error_code(status).to_string(),
        format!("Engine error: {api_error}"),
    );
    match serde_json::to_string(&error_response) {
        Ok(error_json) => (status, error_json).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize error response: {}", e),
        )
            .into_response(),
    }
}

fn api_error_code(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "BAD_REQUEST",
        StatusCode::NOT_FOUND => "NOT_FOUND",
        StatusCode::SERVICE_UNAVAILABLE => "SERVICE_UNAVAILABLE",
        StatusCode::BAD_GATEWAY => "BAD_GATEWAY",
        StatusCode::METHOD_NOT_ALLOWED => "METHOD_NOT_ALLOWED",
        StatusCode::PAYLOAD_TOO_LARGE => "PAYLOAD_TOO_LARGE",
        _ => "INTERNAL_ERROR",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::error::ApiError;
    use probing_proto::protocol::query::{ErrorCode, QueryError};

    #[test]
    fn api_error_code_maps_known_statuses() {
        assert_eq!(api_error_code(StatusCode::BAD_REQUEST), "BAD_REQUEST");
        assert_eq!(api_error_code(StatusCode::NOT_FOUND), "NOT_FOUND");
        assert_eq!(
            api_error_code(StatusCode::SERVICE_UNAVAILABLE),
            "SERVICE_UNAVAILABLE"
        );
        assert_eq!(
            api_error_code(StatusCode::INTERNAL_SERVER_ERROR),
            "INTERNAL_ERROR"
        );
    }

    #[test]
    fn convert_engine_error_preserves_status_code() {
        let err = ApiError::not_found("missing table");
        assert_eq!(err.status(), StatusCode::NOT_FOUND);
        assert_eq!(api_error_code(err.status()), "NOT_FOUND");
    }

    #[test]
    fn engine_error_status_maps_not_found() {
        let err = QueryError {
            code: ErrorCode::NotFound,
            message: "missing table".into(),
            details: None,
        };
        assert_eq!(engine_error_status(&err), StatusCode::NOT_FOUND);
    }
}

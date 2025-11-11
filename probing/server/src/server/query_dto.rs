//! Query DTO handler functions
//!
//! This module contains all the functions related to handling query DTOs,
//! separated from the main server module for better organization.

use axum::http::StatusCode;
use axum::response::IntoResponse;
use probing_proto::protocol::message::Message;
use probing_proto::protocol::query::{Data as ProtoData, Query as ProtoQuery};
use serde_json;

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
        Ok(response_json) => convert_engine_response_to_dto(response_json).await,
        Err(api_error) => convert_engine_error_to_dto(api_error).await,
    }
}

/// Convert engine response to DTO format
async fn convert_engine_response_to_dto(response_json: String) -> axum::response::Response {
    // Parse the response to convert to DTO format
    match serde_json::from_str::<Message<ProtoData>>(&response_json) {
        Ok(message_response) => {
            let response_dto = probing_proto::dto::query::QueryResponseDto::success(
                message_response.payload.into(),
            );

            match serde_json::to_string(&response_dto) {
                Ok(dto_response_json) => (StatusCode::OK, dto_response_json).into_response(),
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

/// Convert engine error to DTO error response
async fn convert_engine_error_to_dto(
    api_error: crate::server::error::ApiError,
) -> axum::response::Response {
    // Convert ApiError to DTO error response
    let error_response = probing_proto::dto::query::QueryResponseDto::error(
        "INTERNAL_ERROR".to_string(),
        format!("Engine error: {}", api_error.0),
    );
    match serde_json::to_string(&error_response) {
        Ok(error_json) => (StatusCode::INTERNAL_SERVER_ERROR, error_json).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to serialize error response: {}", e),
        )
            .into_response(),
    }
}

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    response::Json as ResponseJson,
};
use db::models::chat_message::{ChatMessage, ChatSenderType};
use db::models::chat_session::ChatSession;
use deployment::Deployment;
use serde::Deserialize;
use ts_rs::TS;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Deserialize, TS)]
pub struct ChatMessageListQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateChatMessageRequest {
    pub sender_type: ChatSenderType,
    pub sender_id: Option<Uuid>,
    pub content: String,
    pub meta: Option<serde_json::Value>,
}

pub async fn get_messages(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ChatMessageListQuery>,
) -> Result<ResponseJson<ApiResponse<Vec<ChatMessage>>>, ApiError> {
    let messages =
        ChatMessage::find_by_session_id(&deployment.db().pool, session.id, query.limit).await?;
    Ok(ResponseJson(ApiResponse::success(messages)))
}

pub async fn create_message(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateChatMessageRequest>,
) -> Result<ResponseJson<ApiResponse<ChatMessage>>, ApiError> {
    let message = services::services::chat::create_message(
        &deployment.db().pool,
        session.id,
        payload.sender_type,
        payload.sender_id,
        payload.content,
        payload.meta,
    )
    .await?;

    deployment.chat_runner().handle_message(&session, &message).await;

    Ok(ResponseJson(ApiResponse::success(message)))
}

pub async fn get_message(
    State(deployment): State<DeploymentImpl>,
    Path(message_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<ChatMessage>>, ApiError> {
    let message = ChatMessage::find_by_id(&deployment.db().pool, message_id)
        .await?
        .ok_or(ApiError::Database(sqlx::Error::RowNotFound))?;
    Ok(ResponseJson(ApiResponse::success(message)))
}

pub async fn delete_message(
    State(deployment): State<DeploymentImpl>,
    Path(message_id): Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let rows_affected = ChatMessage::delete(&deployment.db().pool, message_id).await?;
    if rows_affected == 0 {
        Err(ApiError::Database(sqlx::Error::RowNotFound))
    } else {
        Ok(ResponseJson(ApiResponse::success(())))
    }
}

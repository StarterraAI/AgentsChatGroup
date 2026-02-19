use std::path::{Component, PathBuf};

use axum::{
    Extension, Json,
    extract::{Multipart, Path, Query, State},
    http::{StatusCode, header},
    response::{Json as ResponseJson, Response},
};
use db::models::{
    chat_message::{ChatMessage, ChatSenderType},
    chat_session::ChatSession,
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::chat::ChatAttachmentMeta;
use tokio::{fs, fs::File};
use tokio_util::io::ReaderStream;
use ts_rs::TS;
use utils::{assets::asset_dir, response::ApiResponse};
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

const ALLOWED_TEXT_EXTENSIONS: &[&str] = &[
    ".txt", ".csv", ".md", ".json", ".xml", ".yaml", ".yml", ".html", ".htm", ".css", ".js", ".ts",
    ".jsx", ".tsx", ".py", ".java", ".c", ".cpp", ".h", ".hpp", ".rb", ".php", ".go", ".rs",
    ".sql", ".sh", ".bash", ".svg",
];

const ALLOWED_IMAGE_EXTENSIONS: &[&str] =
    &[".png", ".jpg", ".jpeg", ".gif", ".webp", ".bmp", ".svg"];

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

#[derive(Debug, Deserialize, TS)]
#[ts(export)]
pub struct DeleteMessagesRequest {
    pub message_ids: Vec<Uuid>,
}

fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .collect();
    if sanitized.is_empty() {
        "file".to_string()
    } else {
        sanitized.chars().take(120).collect()
    }
}

fn attachment_kind(mime: Option<&str>) -> String {
    if let Some(mime) = mime {
        if mime.starts_with("image/") {
            return "image".to_string();
        }
    }
    "file".to_string()
}

fn is_allowed_attachment(filename: &str, mime: Option<&str>) -> bool {
    if let Some(mime) = mime {
        if mime.starts_with("text/") || mime.starts_with("image/") {
            return true;
        }
    }
    let lower = filename.to_ascii_lowercase();
    ALLOWED_TEXT_EXTENSIONS
        .iter()
        .chain(ALLOWED_IMAGE_EXTENSIONS.iter())
        .any(|ext| lower.ends_with(ext))
}

fn attachment_storage_dir(session_id: Uuid, message_id: Uuid) -> PathBuf {
    asset_dir()
        .join("chat")
        .join(format!("session_{session_id}"))
        .join("attachments")
        .join(message_id.to_string())
}

fn resolve_relative_path(relative_path: &str) -> Option<PathBuf> {
    let rel = PathBuf::from(relative_path);
    if rel.is_absolute() {
        return None;
    }
    if rel
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return None;
    }
    Some(asset_dir().join(rel))
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

    deployment
        .chat_runner()
        .handle_message(&session, &message)
        .await;

    Ok(ResponseJson(ApiResponse::success(message)))
}

pub async fn upload_message_attachments(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    mut multipart: Multipart,
) -> Result<ResponseJson<ApiResponse<ChatMessage>>, ApiError> {
    let message_id = Uuid::new_v4();
    let mut content: Option<String> = None;
    let mut sender_handle: Option<String> = None;
    let mut reference_message_id: Option<Uuid> = None;
    let mut attachments: Vec<ChatAttachmentMeta> = Vec::new();

    while let Some(field) = multipart.next_field().await? {
        match field.name() {
            Some("content") => {
                let text = field.text().await?;
                if !text.trim().is_empty() {
                    content = Some(text);
                }
            }
            Some("sender_handle") => {
                let text = field.text().await?;
                if !text.trim().is_empty() {
                    sender_handle = Some(text);
                }
            }
            Some("reference_message_id") => {
                let text = field.text().await?;
                if let Ok(parsed) = Uuid::parse_str(text.trim()) {
                    reference_message_id = Some(parsed);
                }
            }
            _ => {
                let filename = field.file_name().map(|name| name.to_string());
                let mime_type = field.content_type().map(|value| value.to_string());
                let Some(filename) = filename else {
                    continue;
                };
                if !is_allowed_attachment(&filename, mime_type.as_deref()) {
                    return Err(ApiError::BadRequest(
                        "Only text files and images are allowed.".to_string(),
                    ));
                }
                let data = field.bytes().await?;
                if data.is_empty() {
                    continue;
                }

                let attachment_id = Uuid::new_v4();
                let original_name = filename.to_string();
                let sanitized = sanitize_filename(&filename);
                let stored_name = format!("{attachment_id}_{sanitized}");
                let storage_dir = attachment_storage_dir(session.id, message_id);
                fs::create_dir_all(&storage_dir).await?;
                let storage_path = storage_dir.join(&stored_name);
                fs::write(&storage_path, &data).await?;

                let kind = attachment_kind(mime_type.as_deref());
                let relative_path = format!(
                    "chat/session_{}/attachments/{}/{}",
                    session.id, message_id, stored_name
                );

                attachments.push(ChatAttachmentMeta {
                    id: attachment_id,
                    name: original_name,
                    mime_type,
                    size_bytes: data.len() as i64,
                    kind,
                    relative_path,
                });
            }
        }
    }

    if attachments.is_empty() {
        return Err(ApiError::BadRequest(
            "No attachments were uploaded.".to_string(),
        ));
    }

    let fallback_content = if attachments.len() == 1 {
        format!("Uploaded {}", attachments[0].name)
    } else {
        format!("Uploaded {} files", attachments.len())
    };
    let content = content.unwrap_or(fallback_content);

    let mut meta = serde_json::json!({ "attachments": attachments });
    if let Some(handle) = sender_handle {
        meta["sender_handle"] = serde_json::json!(handle);
    }
    if let Some(reference_id) = reference_message_id {
        meta["reference"] = serde_json::json!({ "message_id": reference_id });
    }

    let message = services::services::chat::create_message_with_id(
        &deployment.db().pool,
        session.id,
        ChatSenderType::User,
        None,
        content,
        Some(meta),
        message_id,
    )
    .await?;

    deployment
        .chat_runner()
        .handle_message(&session, &message)
        .await;

    Ok(ResponseJson(ApiResponse::success(message)))
}

pub async fn serve_message_attachment(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Path((_session_id, message_id, attachment_id)): Path<(Uuid, Uuid, Uuid)>,
) -> Result<Response, ApiError> {
    let message = ChatMessage::find_by_id(&deployment.db().pool, message_id)
        .await?
        .ok_or(ApiError::Database(sqlx::Error::RowNotFound))?;

    if message.session_id != session.id {
        return Err(ApiError::Database(sqlx::Error::RowNotFound));
    }

    let attachments = services::services::chat::extract_attachments(&message.meta.0);
    let attachment = attachments
        .into_iter()
        .find(|item| item.id == attachment_id)
        .ok_or_else(|| ApiError::BadRequest("Attachment not found".to_string()))?;

    let Some(path) = resolve_relative_path(&attachment.relative_path) else {
        return Err(ApiError::BadRequest("Invalid attachment path".to_string()));
    };

    let file = File::open(&path).await?;
    let metadata = file.metadata().await?;
    let stream = ReaderStream::new(file);
    let body = axum::body::Body::from_stream(stream);

    let content_type = attachment
        .mime_type
        .as_deref()
        .unwrap_or("application/octet-stream");

    let header_name = sanitize_filename(&attachment.name);
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CONTENT_LENGTH, metadata.len())
        .header(
            header::CONTENT_DISPOSITION,
            format!("inline; filename=\"{}\"", header_name),
        )
        .body(body)
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    Ok(response)
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

/// Delete multiple messages at once
pub async fn delete_messages_batch(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<DeleteMessagesRequest>,
) -> Result<ResponseJson<ApiResponse<u64>>, ApiError> {
    if payload.message_ids.is_empty() {
        return Ok(ResponseJson(ApiResponse::success(0)));
    }

    let mut total_deleted: u64 = 0;
    for message_id in payload.message_ids {
        // Verify the message belongs to this session before deleting
        if let Some(message) = ChatMessage::find_by_id(&deployment.db().pool, message_id).await? {
            if message.session_id == session.id {
                let rows = ChatMessage::delete(&deployment.db().pool, message_id).await?;
                total_deleted += rows;
            }
        }
    }

    Ok(ResponseJson(ApiResponse::success(total_deleted)))
}

use axum::{
    Extension, Json,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::{IntoResponse, Json as ResponseJson},
};
use db::models::chat_session::{
    ChatSession, ChatSessionStatus, CreateChatSession, UpdateChatSession,
};
use db::models::chat_session_agent::{
    ChatSessionAgent, CreateChatSessionAgent,
};
use deployment::Deployment;
use serde::Deserialize;
use ts_rs::TS;
use utils::response::ApiResponse;
use utils::assets::asset_dir;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

#[derive(Debug, Deserialize, TS)]
pub struct ChatSessionListQuery {
    pub status: Option<ChatSessionStatus>,
}

pub async fn get_sessions(
    State(deployment): State<DeploymentImpl>,
    Query(query): Query<ChatSessionListQuery>,
) -> Result<ResponseJson<ApiResponse<Vec<ChatSession>>>, ApiError> {
    let sessions = ChatSession::find_all(&deployment.db().pool, query.status).await?;
    Ok(ResponseJson(ApiResponse::success(sessions)))
}

pub async fn get_session(
    Extension(session): Extension<ChatSession>,
) -> Result<ResponseJson<ApiResponse<ChatSession>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(session)))
}

pub async fn create_session(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateChatSession>,
) -> Result<ResponseJson<ApiResponse<ChatSession>>, ApiError> {
    let session = ChatSession::create(&deployment.db().pool, &payload, Uuid::new_v4()).await?;
    Ok(ResponseJson(ApiResponse::success(session)))
}

pub async fn update_session(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<UpdateChatSession>,
) -> Result<ResponseJson<ApiResponse<ChatSession>>, ApiError> {
    let updated = ChatSession::update(&deployment.db().pool, session.id, &payload).await?;
    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn delete_session(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let rows_affected = ChatSession::delete(&deployment.db().pool, session.id).await?;
    if rows_affected == 0 {
        Err(ApiError::Database(sqlx::Error::RowNotFound))
    } else {
        Ok(ResponseJson(ApiResponse::success(())))
    }
}

#[derive(Debug, Deserialize, TS)]
pub struct CreateChatSessionAgentRequest {
    pub agent_id: Uuid,
    pub workspace_path: Option<String>,
}

#[derive(Debug, Deserialize, TS)]
pub struct UpdateChatSessionAgentRequest {
    pub workspace_path: Option<String>,
}

pub async fn get_session_agents(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<ChatSessionAgent>>>, ApiError> {
    let agents =
        ChatSessionAgent::find_all_for_session(&deployment.db().pool, session.id).await?;
    Ok(ResponseJson(ApiResponse::success(agents)))
}

pub async fn create_session_agent(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateChatSessionAgentRequest>,
) -> Result<ResponseJson<ApiResponse<ChatSessionAgent>>, ApiError> {
    if session.status != ChatSessionStatus::Active {
        return Err(ApiError::Conflict(
            "Chat session is archived".to_string(),
        ));
    }

    if let Some(existing) = ChatSessionAgent::find_by_session_and_agent(
        &deployment.db().pool,
        session.id,
        payload.agent_id,
    )
    .await?
    {
        if payload.workspace_path.is_some() {
            let updated = ChatSessionAgent::update_workspace_path(
                &deployment.db().pool,
                existing.id,
                payload.workspace_path,
            )
            .await?;
            return Ok(ResponseJson(ApiResponse::success(updated)));
        }
        return Ok(ResponseJson(ApiResponse::success(existing)));
    }

    let created = ChatSessionAgent::create(
        &deployment.db().pool,
        &CreateChatSessionAgent {
            session_id: session.id,
            agent_id: payload.agent_id,
            workspace_path: payload.workspace_path,
        },
        Uuid::new_v4(),
    )
    .await?;
    Ok(ResponseJson(ApiResponse::success(created)))
}

pub async fn update_session_agent(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    axum::extract::Path(session_agent_id): axum::extract::Path<Uuid>,
    Json(payload): Json<UpdateChatSessionAgentRequest>,
) -> Result<ResponseJson<ApiResponse<ChatSessionAgent>>, ApiError> {
    if session.status != ChatSessionStatus::Active {
        return Err(ApiError::Conflict(
            "Chat session is archived".to_string(),
        ));
    }

    let Some(existing) =
        ChatSessionAgent::find_by_id(&deployment.db().pool, session_agent_id).await?
    else {
        return Err(ApiError::BadRequest("Chat session agent not found".to_string()));
    };

    if existing.session_id != session.id {
        return Err(ApiError::Forbidden(
            "Chat session agent does not belong to this session".to_string(),
        ));
    }

    let updated = ChatSessionAgent::update_workspace_path(
        &deployment.db().pool,
        existing.id,
        payload.workspace_path,
    )
    .await?;
    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn delete_session_agent(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    axum::extract::Path(session_agent_id): axum::extract::Path<Uuid>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let Some(existing) =
        ChatSessionAgent::find_by_id(&deployment.db().pool, session_agent_id).await?
    else {
        return Err(ApiError::BadRequest("Chat session agent not found".to_string()));
    };

    if existing.session_id != session.id {
        return Err(ApiError::Forbidden(
            "Chat session agent does not belong to this session".to_string(),
        ));
    }

    let rows = ChatSessionAgent::delete(&deployment.db().pool, existing.id).await?;
    if rows == 0 {
        Err(ApiError::BadRequest("Chat session agent not found".to_string()))
    } else {
        Ok(ResponseJson(ApiResponse::success(())))
    }
}

pub async fn archive_session(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<ChatSession>>, ApiError> {
    if session.status == ChatSessionStatus::Archived {
        return Ok(ResponseJson(ApiResponse::success(session)));
    }

    let archive_dir = asset_dir()
        .join("chat")
        .join(format!("session_{}", session.id))
        .join("archive");
    let archive_ref = services::services::chat::export_session_archive(
        &deployment.db().pool,
        &session,
        archive_dir.as_path(),
    )
    .await?;

    let updated = ChatSession::update(
        &deployment.db().pool,
        session.id,
        &UpdateChatSession {
            title: None,
            status: Some(ChatSessionStatus::Archived),
            summary_text: None,
            archive_ref: Some(archive_ref),
        },
    )
    .await?;

    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn restore_session(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<ChatSession>>, ApiError> {
    if session.status == ChatSessionStatus::Active {
        return Ok(ResponseJson(ApiResponse::success(session)));
    }

    let updated = ChatSession::update(
        &deployment.db().pool,
        session.id,
        &UpdateChatSession {
            title: None,
            status: Some(ChatSessionStatus::Active),
            summary_text: None,
            archive_ref: None,
        },
    )
    .await?;

    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn stream_session_ws(
    ws: WebSocketUpgrade,
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<impl IntoResponse, ApiError> {
    let rx = deployment.chat_runner().subscribe(session.id);

    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(err) = handle_chat_stream_ws(socket, rx).await {
            tracing::warn!("chat stream ws closed: {}", err);
        }
    }))
}

async fn handle_chat_stream_ws(
    socket: WebSocket,
    mut rx: tokio::sync::broadcast::Receiver<
        services::services::chat_runner::ChatStreamEvent,
    >,
) -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};

    let (mut sender, mut receiver) = socket.split();
    tokio::spawn(async move { while let Some(Ok(_)) = receiver.next().await {} });

    loop {
        match rx.recv().await {
            Ok(event) => {
                let json = serde_json::to_string(&event)?;
                if sender.send(Message::Text(json.into())).await.is_err() {
                    break;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
        }
    }

    Ok(())
}

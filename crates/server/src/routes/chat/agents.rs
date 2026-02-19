use axum::{Extension, Json, extract::State, response::Json as ResponseJson};
use db::models::{
    chat_agent::{ChatAgent, CreateChatAgent, UpdateChatAgent},
    chat_session_agent::ChatSessionAgent,
};
use deployment::Deployment;
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub async fn get_agents(
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<ChatAgent>>>, ApiError> {
    let agents = ChatAgent::find_all(&deployment.db().pool).await?;
    Ok(ResponseJson(ApiResponse::success(agents)))
}

pub async fn get_agent(
    Extension(agent): Extension<ChatAgent>,
) -> Result<ResponseJson<ApiResponse<ChatAgent>>, ApiError> {
    Ok(ResponseJson(ApiResponse::success(agent)))
}

pub async fn create_agent(
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<CreateChatAgent>,
) -> Result<ResponseJson<ApiResponse<ChatAgent>>, ApiError> {
    let agent = ChatAgent::create(&deployment.db().pool, &payload, Uuid::new_v4()).await?;
    Ok(ResponseJson(ApiResponse::success(agent)))
}

pub async fn update_agent(
    Extension(agent): Extension<ChatAgent>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<UpdateChatAgent>,
) -> Result<ResponseJson<ApiResponse<ChatAgent>>, ApiError> {
    // Check if runner_type is being changed
    let runner_type_changing = payload
        .runner_type
        .as_ref()
        .is_some_and(|new_type| new_type != &agent.runner_type);

    let updated = ChatAgent::update(&deployment.db().pool, agent.id, &payload).await?;

    // If runner_type changed, clear the agent_session_id and agent_message_id
    // from all ChatSessionAgent records using this agent, as the old session IDs
    // are no longer valid for the new model.
    if runner_type_changing {
        if let Err(err) =
            ChatSessionAgent::clear_session_ids_for_agent(&deployment.db().pool, agent.id).await
        {
            tracing::warn!(
                agent_id = %agent.id,
                error = %err,
                "Failed to clear session IDs after runner_type change"
            );
        }
    }

    Ok(ResponseJson(ApiResponse::success(updated)))
}

pub async fn delete_agent(
    Extension(agent): Extension<ChatAgent>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let rows_affected = ChatAgent::delete(&deployment.db().pool, agent.id).await?;
    if rows_affected == 0 {
        Err(ApiError::Database(sqlx::Error::RowNotFound))
    } else {
        Ok(ResponseJson(ApiResponse::success(())))
    }
}

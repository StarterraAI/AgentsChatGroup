use axum::{
    Extension, Json,
    extract::State,
    response::Json as ResponseJson,
};
use db::models::chat_agent::{ChatAgent, CreateChatAgent, UpdateChatAgent};
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
    let updated = ChatAgent::update(&deployment.db().pool, agent.id, &payload).await?;
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

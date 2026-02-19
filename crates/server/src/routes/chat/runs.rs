use std::path::PathBuf;

use axum::{
    extract::{Path, Query, State},
    http::header::CONTENT_TYPE,
    response::{IntoResponse, Response},
};
use db::models::chat_run::ChatRun;
use deployment::Deployment;
use serde::Deserialize;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub async fn get_run_log(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let Some(run) = ChatRun::find_by_id(&deployment.db().pool, run_id).await? else {
        return Err(ApiError::BadRequest("Chat run not found".to_string()));
    };

    let Some(log_path) = run.raw_log_path else {
        return Err(ApiError::BadRequest("Chat run has no log".to_string()));
    };

    let content = match tokio::fs::read_to_string(&log_path).await {
        Ok(content) => content,
        Err(_) => {
            return Err(ApiError::BadRequest(
                "Chat run log file not found".to_string(),
            ));
        }
    };

    Ok(([(CONTENT_TYPE, "text/plain; charset=utf-8")], content).into_response())
}

pub async fn get_run_diff(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let Some(run) = ChatRun::find_by_id(&deployment.db().pool, run_id).await? else {
        return Err(ApiError::BadRequest("Chat run not found".to_string()));
    };

    let diff_path = PathBuf::from(run.run_dir).join("diff.patch");
    let content = match tokio::fs::read_to_string(&diff_path).await {
        Ok(content) => content,
        Err(_) => {
            return Err(ApiError::BadRequest(
                "Chat run diff file not found".to_string(),
            ));
        }
    };

    Ok(([(CONTENT_TYPE, "text/plain; charset=utf-8")], content).into_response())
}

#[derive(Debug, Deserialize)]
pub struct UntrackedFileQuery {
    path: String,
}

pub async fn get_run_untracked_file(
    State(deployment): State<DeploymentImpl>,
    Path(run_id): Path<Uuid>,
    Query(query): Query<UntrackedFileQuery>,
) -> Result<Response, ApiError> {
    let Some(run) = ChatRun::find_by_id(&deployment.db().pool, run_id).await? else {
        return Err(ApiError::BadRequest("Chat run not found".to_string()));
    };

    let rel_path = PathBuf::from(&query.path);
    if rel_path.is_absolute()
        || rel_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(ApiError::BadRequest("Invalid untracked path".to_string()));
    }

    let file_path = PathBuf::from(run.run_dir).join("untracked").join(rel_path);
    let content = match tokio::fs::read_to_string(&file_path).await {
        Ok(content) => content,
        Err(_) => {
            return Err(ApiError::BadRequest(
                "Untracked file content not found".to_string(),
            ));
        }
    };

    Ok(([(CONTENT_TYPE, "text/plain; charset=utf-8")], content).into_response())
}

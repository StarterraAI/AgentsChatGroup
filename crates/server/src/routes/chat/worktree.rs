use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    response::Json as ResponseJson,
    routing::{get, post},
};
use db::models::{
    chat_session::{ChatSession, ChatSessionWorktreeMode},
    chat_session_worktree::{SessionWorktree, SessionWorktreeMergeOperation},
};
use deployment::Deployment;
use serde::Deserialize;
use services::services::session_worktree::{
    ConflictFileContent, ConflictFileInfo, ConflictResolutionSide, EnsureOutcome,
    EnsureWorktreeInput, MergeResult, SessionWorktreeError, SessionWorktreeService,
};
use utils::response::ApiResponse;
use uuid::Uuid;

use crate::{DeploymentImpl, error::ApiError};

pub fn router() -> Router<DeploymentImpl> {
    Router::new()
        .route("/", get(get_worktree_status).post(prepare_worktree))
        .route("/merge", post(merge_worktree))
        .route("/discard", post(discard_worktree))
        .route("/cleanup", post(cleanup_merged_worktree))
        .route("/retry-cleanup", post(retry_cleanup_worktree))
        .route("/force-remove", post(force_remove_worktree))
        .route("/merge-conflicts", get(list_merge_conflicts))
        .route(
            "/merge-conflicts/{*file_path}",
            get(get_merge_conflict_detail),
        )
        .route("/merge-conflicts/resolve", post(resolve_merge_conflict))
        .route("/resolve", post(resolve_merge_conflict))
        .route("/merge/continue", post(continue_merge))
        .route("/merge/abort", post(abort_merge))
}

// -----------------------------------------------------------------
// Request / response payloads
// -----------------------------------------------------------------

#[derive(Debug, Default, Deserialize, ts_rs::TS)]
pub struct PrepareWorktreeRequest {
    #[serde(default)]
    #[ts(optional, type = "string | null")]
    pub base_workspace_path: Option<String>,
}

#[derive(Debug, Default, Deserialize, ts_rs::TS)]
pub struct MergeWorktreeRequest {
    #[serde(default)]
    #[ts(optional, type = "string | null")]
    pub commit_message: Option<String>,
    #[serde(default)]
    #[ts(optional, type = "string | null")]
    pub target_branch: Option<String>,
}

#[derive(Debug, Default, Deserialize, ts_rs::TS)]
pub struct ResolveConflictRequest {
    /// Relative path of the conflicted file to resolve, e.g. `src/main.rs`.
    /// Moved from a path parameter to the body so Axum's catch-all
    /// limitation (must be last segment) doesn't block nested paths in
    /// the resolve endpoint.
    pub path: String,
    #[serde(default)]
    #[ts(optional, type = "string | null")]
    pub content: Option<String>,
    #[serde(default)]
    #[ts(optional)]
    pub use_stage: Option<ConflictResolutionSide>,
    #[serde(default)]
    #[ts(optional)]
    pub delete_file: Option<bool>,
}

#[derive(Debug, Default, Deserialize, ts_rs::TS)]
pub struct ContinueMergeRequest {
    #[serde(default)]
    #[ts(optional, type = "string | null")]
    pub commit_message: Option<String>,
}

// -----------------------------------------------------------------
// Route handlers
// -----------------------------------------------------------------

/// GET /chat/sessions/{session_id}/worktree
/// Returns the most recent session worktree row regardless of status, or null
/// if no worktree exists. This includes terminal states (`merged`,
/// `cleanup_failed`, `archived`) so the UI can render read-only history and
/// expose the retry-cleanup entry point.
pub async fn get_worktree_status(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Option<SessionWorktree>>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .get_latest_for_session(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// POST /chat/sessions/{session_id}/worktree
/// Lazily create (or return the existing) isolated worktree for the session.
/// Only sessions with `worktree_mode == isolated` may create a worktree;
/// `inherit` / `disabled` sessions are rejected to preserve the main-workspace
/// behavior for sessions that have not opted into isolation.
pub async fn prepare_worktree(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<PrepareWorktreeRequest>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    if session.worktree_mode != ChatSessionWorktreeMode::Isolated {
        return Err(ApiError::BadRequest(format!(
            "Session worktree_mode is {0:?}, not 'isolated'. Enable isolated worktree mode before preparing a worktree.",
            session.worktree_mode
        )));
    }

    let service = SessionWorktreeService::new(deployment.db().pool.clone());

    let base_workspace_path = payload
        .base_workspace_path
        .or(session.default_workspace_path)
        .ok_or_else(|| {
            ApiError::BadRequest(
                "Session has no default workspace path; provide base_workspace_path.".to_string(),
            )
        })?;

    let input = EnsureWorktreeInput::new(session.id, base_workspace_path.into())
        .with_project(session.project_id);

    let outcome = service
        .ensure_for_session(input)
        .await
        .map_err(session_worktree_api_error)?;

    let worktree = match outcome {
        EnsureOutcome::Created(w) => w,
        EnsureOutcome::Existing(w) => w,
    };
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// POST /chat/sessions/{session_id}/worktree/merge
/// Merge session worktree changes into the base workspace.
pub async fn merge_worktree(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<MergeWorktreeRequest>,
) -> Result<ResponseJson<ApiResponse<MergeResult>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let result = service
        .perform_merge(
            session.id,
            // Integrate the session branch as one combined change. This keeps
            // the base history free of a synthetic two-parent merge commit
            // without forcing the user to resolve the same area once per
            // session commit.
            SessionWorktreeMergeOperation::SquashMerge,
            payload.target_branch,
            payload.commit_message,
        )
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(result)))
}

/// POST /chat/sessions/{session_id}/worktree/discard
/// User-initiated discard of the session worktree (force-remove).
pub async fn discard_worktree(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .discard_worktree(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// POST /chat/sessions/{session_id}/worktree/cleanup
/// Background cleanup of a merged worktree.
pub async fn cleanup_merged_worktree(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .cleanup_merged_worktree(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// POST /chat/sessions/{session_id}/worktree/retry-cleanup
/// Retry a previously failed cleanup.
pub async fn retry_cleanup_worktree(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .retry_cleanup(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// POST /chat/sessions/{session_id}/worktree/force-remove
/// Force-remove a cleanup_failed worktree from Git/app state.
pub async fn force_remove_worktree(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .force_remove_failed_cleanup(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// GET /chat/sessions/{session_id}/worktree/merge-conflicts
/// List conflicted (unmerged) files from the base workspace's Git index.
pub async fn list_merge_conflicts(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<Vec<ConflictFileInfo>>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let files = service
        .list_conflict_files(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(files)))
}

/// GET /chat/sessions/{session_id}/worktree/merge-conflicts/{*file_path}
/// Read three-way conflict content from Git index stages for a single file.
/// Uses a catch-all path segment so nested paths like `src/main.rs` match.
pub async fn get_merge_conflict_detail(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Path((_session_id, file_path)): Path<(Uuid, String)>,
) -> Result<ResponseJson<ApiResponse<ConflictFileContent>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let content = service
        .read_conflict_file(session.id, &file_path)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(content)))
}

/// POST /chat/sessions/{session_id}/worktree/merge-conflicts/resolve
/// Write resolved content and `git add` the file. The file path is in the
/// request body (not a path parameter) so nested paths like `src/main.rs`
/// work without Axum catch-all limitations.
pub async fn resolve_merge_conflict(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<ResolveConflictRequest>,
) -> Result<ResponseJson<ApiResponse<()>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    service
        .resolve_conflict_file(
            session.id,
            &payload.path,
            payload.content.as_deref(),
            payload.use_stage,
            payload.delete_file.unwrap_or(false),
        )
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(())))
}

/// POST /chat/sessions/{session_id}/worktree/merge/continue
/// Complete the merge after all conflicts have been resolved.
pub async fn continue_merge(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
    Json(payload): Json<ContinueMergeRequest>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .continue_merge(session.id, payload.commit_message)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

/// POST /chat/sessions/{session_id}/worktree/merge/abort
/// Abort the in-progress merge, preserving the session worktree.
pub async fn abort_merge(
    Extension(session): Extension<ChatSession>,
    State(deployment): State<DeploymentImpl>,
) -> Result<ResponseJson<ApiResponse<SessionWorktree>>, ApiError> {
    let service = SessionWorktreeService::new(deployment.db().pool.clone());
    let worktree = service
        .perform_abort_merge(session.id)
        .await
        .map_err(session_worktree_api_error)?;
    Ok(ResponseJson(ApiResponse::success(worktree)))
}

// -----------------------------------------------------------------
// Error mapping
// -----------------------------------------------------------------

/// Map `SessionWorktreeError` to `ApiError` with appropriate HTTP status
/// codes, mirroring the `source_control_api_error` precedent.
pub fn session_worktree_api_error(err: SessionWorktreeError) -> ApiError {
    match err {
        SessionWorktreeError::Database(e) => ApiError::Database(e),
        SessionWorktreeError::Io(e) => ApiError::Io(e),
        SessionWorktreeError::WorktreeManager(e) => ApiError::Worktree(e),
        SessionWorktreeError::ModelCas(e) => {
            ApiError::Conflict(format!("Worktree state transition rejected: {e}"))
        }
        SessionWorktreeError::NoActiveWorktree(sid) => {
            ApiError::BadRequest(format!("Session {sid} has no active worktree."))
        }
        SessionWorktreeError::NoMergedWorktree(sid) => {
            ApiError::BadRequest(format!("Session {sid} has no merged worktree to clean up."))
        }
        SessionWorktreeError::MergedCleanupRequiresDiscard(_) => ApiError::BadRequest(
            "Merged worktrees are preserved after merge; use discard worktree to remove the path."
                .to_string(),
        ),
        SessionWorktreeError::NoCleanupFailedWorktree(sid) => {
            ApiError::BadRequest(format!("Session {sid} has no failed cleanup to retry."))
        }
        SessionWorktreeError::SessionHasActiveWorktree(sid) => ApiError::Conflict(format!(
            "Session {sid} still has an active worktree; resolve it before cleanup."
        )),
        SessionWorktreeError::IllegalTransition {
            session_id,
            from,
            to,
            ..
        } => ApiError::Conflict(format!(
            "Cannot transition session {session_id} worktree from {from} to {to}."
        )),
        SessionWorktreeError::NotAGitRepo(path) => {
            ApiError::BadRequest(format!("Not a git repository: {}", path.display()))
        }
        SessionWorktreeError::BaseWorkspaceWrongBranch { expected, actual } => {
            ApiError::Conflict(format!(
                "Base workspace is on '{actual}', expected '{expected}'. Switch to the base branch before merging."
            ))
        }
        SessionWorktreeError::MergeOperationInProgress => ApiError::Conflict(
            "A git merge or rebase is already in progress in the base workspace.".to_string(),
        ),
        SessionWorktreeError::NoMergeInProgress(sid) => {
            ApiError::BadRequest(format!("Session {sid} has no merge in progress."))
        }
        SessionWorktreeError::UnresolvedConflicts(files) => {
            ApiError::Conflict(format!("Unresolved conflicts remain: {}", files.join(", ")))
        }
        SessionWorktreeError::GitCommand(msg) => {
            ApiError::BadRequest(format!("Git operation failed: {msg}"))
        }
        SessionWorktreeError::InvalidConflictPath(path) => {
            ApiError::BadRequest(format!("Invalid conflict file path: {path}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::router;

    #[test]
    fn worktree_router_builds() {
        let _router = router();
    }
}

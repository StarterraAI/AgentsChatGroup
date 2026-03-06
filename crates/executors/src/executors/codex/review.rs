use std::sync::Arc;

use codex_app_server_protocol::{ReviewTarget, ThreadResumeParams, ThreadStartParams};

use super::{client::AppServerClient, session::SessionHandler};
use crate::executors::ExecutorError;

pub async fn launch_codex_review(
    thread_params: ThreadStartParams,
    resume_session: Option<String>,
    review_target: ReviewTarget,
    client: Arc<AppServerClient>,
) -> Result<(), ExecutorError> {
    let auth_status = client.get_auth_status().await?;
    if auth_status.requires_openai_auth.unwrap_or(true) && auth_status.auth_method.is_none() {
        return Err(ExecutorError::AuthRequired(
            "Codex authentication required".to_string(),
        ));
    }

    let thread_id = match resume_session {
        Some(session_id) => {
            let (rollout_path, _forked_session_id) = SessionHandler::fork_rollout_file(&session_id)
                .map_err(|e| ExecutorError::FollowUpNotSupported(e.to_string()))?;
            let params = ThreadResumeParams {
                thread_id: session_id,
                path: Some(rollout_path.clone()),
                model: thread_params.model,
                model_provider: thread_params.model_provider,
                cwd: thread_params.cwd,
                approval_policy: thread_params.approval_policy,
                sandbox: thread_params.sandbox,
                config: thread_params.config,
                base_instructions: thread_params.base_instructions,
                developer_instructions: thread_params.developer_instructions,
                history: None,
            };
            let response = client.resume_thread(params).await?;
            tracing::debug!(
                "resuming session for review using rollout file {}, response {:?}",
                rollout_path.display(),
                response
            );
            response.thread.id
        }
        None => {
            let response = client.start_thread(thread_params).await?;
            response.thread.id
        }
    };

    client.register_session(&thread_id).await?;

    client.start_review(thread_id, review_target).await?;

    Ok(())
}

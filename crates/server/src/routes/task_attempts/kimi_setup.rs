use db::models::{
    execution_process::{ExecutionProcess, ExecutionProcessRunReason},
    session::{CreateSession, Session},
    workspace::{Workspace, WorkspaceError},
};
use deployment::Deployment;
use executors::{actions::ExecutorAction, executors::kimi::KimiCode};
use services::services::container::ContainerService;
use uuid::Uuid;

use crate::error::ApiError;

pub async fn run_kimi_setup(
    deployment: &crate::DeploymentImpl,
    workspace: &Workspace,
    _kimi: &KimiCode,
) -> Result<ExecutionProcess, ApiError> {
    let latest_process = ExecutionProcess::find_latest_by_workspace_and_run_reason(
        &deployment.db().pool,
        workspace.id,
        &ExecutionProcessRunReason::CodingAgent,
    )
    .await?;

    let executor_action = if let Some(latest_process) = latest_process {
        let latest_action = latest_process
            .executor_action()
            .map_err(|e| ApiError::Workspace(WorkspaceError::ValidationError(e.to_string())))?;
        get_setup_helper_action()
            .await?
            .append_action(latest_action.to_owned())
    } else {
        get_setup_helper_action().await?
    };

    deployment
        .container()
        .ensure_container_exists(workspace)
        .await?;

    let session =
        match Session::find_latest_by_workspace_id(&deployment.db().pool, workspace.id).await? {
            Some(s) => s,
            None => {
                Session::create(
                    &deployment.db().pool,
                    &CreateSession {
                        executor: Some("kimi_code".to_string()),
                    },
                    Uuid::new_v4(),
                    workspace.id,
                )
                .await?
            }
        };

    let execution_process = deployment
        .container()
        .start_execution(
            workspace,
            &session,
            &executor_action,
            &ExecutionProcessRunReason::SetupScript,
        )
        .await?;

    Ok(execution_process)
}

async fn get_setup_helper_action() -> Result<ExecutorAction, ApiError> {
    #[cfg(unix)]
    {
        use executors::actions::{
            ExecutorActionType,
            script::{ScriptContext, ScriptRequest, ScriptRequestLanguage},
        };

        let base_command = KimiCode::base_command();

        let install_script = format!(
            r#"#!/bin/bash
set -e
if ! command -v {base_command} &> /dev/null; then
    echo "Installing Kimi Code CLI..."
    curl -LsSf https://code.kimi.com/install.sh | bash
    echo "Installation complete!"
else
    echo "Kimi Code CLI already installed"
fi"#
        );

        let install_request = ScriptRequest {
            script: install_script,
            language: ScriptRequestLanguage::Bash,
            context: ScriptContext::ToolInstallScript,
            working_dir: None,
        };

        let login_script = format!(
            r#"#!/bin/bash
set -e
export PATH="$HOME/.local/bin:$PATH"
{base_command} login
"#
        );

        let login_request = ScriptRequest {
            script: login_script,
            language: ScriptRequestLanguage::Bash,
            context: ScriptContext::ToolInstallScript,
            working_dir: None,
        };

        Ok(ExecutorAction::new(
            ExecutorActionType::ScriptRequest(install_request),
            Some(Box::new(ExecutorAction::new(
                ExecutorActionType::ScriptRequest(login_request),
                None,
            ))),
        ))
    }

    #[cfg(not(unix))]
    {
        use executors::executors::ExecutorError::SetupHelperNotSupported;
        Err(ApiError::Executor(SetupHelperNotSupported))
    }
}

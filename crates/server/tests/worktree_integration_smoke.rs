use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use db::DBService;
use serde_json::{Value, json};
use sqlx::SqlitePool;
use tower::ServiceExt;

fn git(repo: &Path, args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .with_context(|| format!("git {:?} in {}", args, repo.display()))?;

    if !output.status.success() {
        bail!(
            "git {:?} failed in {}\nstdout={}\nstderr={}",
            args,
            repo.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn init_repo(root: &Path) -> Result<()> {
    fs::create_dir_all(root)?;
    let init = Command::new("git")
        .arg("init")
        .arg("-b")
        .arg("main")
        .arg(root)
        .output()?;
    if !init.status.success() {
        bail!(
            "git init failed\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&init.stdout),
            String::from_utf8_lossy(&init.stderr)
        );
    }
    git(root, &["config", "user.email", "smoke@example.test"])?;
    git(root, &["config", "user.name", "OpenTeams Smoke"])?;
    fs::write(root.join("notes.txt"), "base notes\n")?;
    fs::write(root.join("conflict.txt"), "base\n")?;
    git(root, &["add", "."])?;
    git(root, &["commit", "-m", "initial"])?;
    Ok(())
}

async fn test_app() -> Result<Router> {
    let pool = SqlitePool::connect("sqlite::memory:").await?;
    sqlx::migrate!("../db/migrations").run(&pool).await?;
    let deployment =
        local_deployment::LocalDeployment::new_for_test_pool(DBService { pool: pool.clone() })
            .await?;

    Ok(Router::new().nest(
        "/api",
        Router::new()
            .merge(server::routes::projects::router())
            .merge(server::routes::project_source_control::router())
            .merge(server::routes::chat::router(&deployment))
            .with_state(deployment),
    ))
}

async fn api(
    app: &Router,
    method: Method,
    uri: impl Into<String>,
    body: Option<Value>,
) -> Result<(StatusCode, Value)> {
    let uri = uri.into();
    let mut builder = Request::builder().method(method).uri(uri.clone());
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let request = builder.body(match body {
        Some(value) => Body::from(value.to_string()),
        None => Body::empty(),
    })?;
    let response = app.clone().oneshot(request).await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX).await?;
    let text = String::from_utf8_lossy(&bytes);
    let value = if text.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str(&text).with_context(|| format!("decode {status}: {text}"))?
    };
    Ok((status, value))
}

fn data(value: &Value) -> Result<&Value> {
    if value.get("success").and_then(Value::as_bool) != Some(true) {
        bail!("api returned failure: {value}");
    }
    value
        .get("data")
        .ok_or_else(|| anyhow!("missing data: {value}"))
}

fn str_field<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing string field {key}: {value}"))
}

async fn create_session(
    app: &Router,
    project_id: &str,
    title: &str,
    workspace: &Path,
    mode: Option<&str>,
) -> Result<Value> {
    let mut payload = json!({
        "title": title,
        "workspace_path": workspace.to_string_lossy(),
    });
    if let Some(mode) = mode {
        payload["worktree_mode"] = json!(mode);
    }

    let (status, response) = api(
        app,
        Method::POST,
        format!("/api/projects/{project_id}/sessions"),
        Some(payload),
    )
    .await?;
    if status != StatusCode::OK {
        bail!("create session failed {status}: {response}");
    }
    Ok(data(&response)?.clone())
}

async fn prepare_worktree(app: &Router, session_id: &str) -> Result<Value> {
    let (status, response) = api(
        app,
        Method::POST,
        format!("/api/chat/sessions/{session_id}/worktree"),
        Some(json!({})),
    )
    .await?;
    if status != StatusCode::OK {
        bail!("prepare worktree failed {status}: {response}");
    }
    Ok(data(&response)?.clone())
}

async fn source_workspace(app: &Router, project_id: &str, session_id: &str) -> Result<String> {
    let (status, response) = api(
        app,
        Method::GET,
        format!("/api/projects/{project_id}/source-control/session-status?session_id={session_id}"),
        None,
    )
    .await?;
    if status != StatusCode::OK {
        bail!("source-control status failed {status}: {response}");
    }
    Ok(str_field(data(&response)?, "workspace_path")?.to_string())
}

#[tokio::test]
async fn session_worktree_routes_cover_main_merge_conflict_and_cleanup_flows() -> Result<()> {
    let app = test_app().await?;
    let temp = tempfile::TempDir::new()?;
    let base = temp.path().join("base");
    init_repo(&base)?;
    let base_string = base.to_string_lossy().to_string();

    let (status, project_response) = api(
        &app,
        Method::POST,
        "/api/projects",
        Some(json!({
            "name": "worktree smoke",
            "repositories": [],
            "description": null,
            "status": null,
            "default_workspace_path": base_string,
            "active_repo_id": null
        })),
    )
    .await?;
    if status != StatusCode::OK {
        bail!("create project failed {status}: {project_response}");
    }
    let project_id = str_field(data(&project_response)?, "id")?;

    let disabled =
        create_session(&app, project_id, "main workspace", &base, Some("disabled")).await?;
    let disabled_id = str_field(&disabled, "id")?;
    assert_eq!(
        source_workspace(&app, project_id, disabled_id).await?,
        base_string
    );
    let (reject_status, _) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{disabled_id}/worktree"),
        Some(json!({})),
    )
    .await?;
    assert_eq!(reject_status, StatusCode::BAD_REQUEST);

    let clean = create_session(&app, project_id, "clean merge", &base, Some("isolated")).await?;
    let clean_id = str_field(&clean, "id")?;
    let clean_worktree = prepare_worktree(&app, clean_id).await?;
    let clean_path = PathBuf::from(str_field(&clean_worktree, "worktree_path")?);
    assert!(clean_path.exists());
    assert_ne!(clean_path, base);
    fs::write(clean_path.join("session.txt"), "from isolated worktree\n")?;
    git(&clean_path, &["add", "session.txt"])?;
    git(&clean_path, &["commit", "-m", "smoke session change"])?;
    fs::create_dir_all(base.join(".openteams/context"))?;
    fs::write(
        base.join(".openteams/context/runtime.jsonl"),
        "{\"event\":\"base runtime\"}\n",
    )?;
    fs::create_dir_all(clean_path.join(".openteams/context"))?;
    fs::write(
        clean_path.join(".openteams/context/runtime.jsonl"),
        "{\"event\":\"worktree runtime\"}\n",
    )?;
    assert_eq!(
        source_workspace(&app, project_id, clean_id).await?,
        clean_path.to_string_lossy()
    );

    let (status, merge_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{clean_id}/worktree/merge"),
        Some(json!({ "commit_message": "smoke clean merge" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{merge_response}");
    assert_eq!(
        data(&merge_response)?
            .get("has_conflicts")
            .and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        str_field(
            data(&merge_response)?.get("worktree").unwrap(),
            "merge_operation"
        )?,
        "squash_merge"
    );
    assert!(base.join("session.txt").exists());
    assert_eq!(git(&base, &["ls-files", ".openteams"])?, "");
    // Merged worktrees stay active until an explicit discard, so the session
    // source-control workspace remains the worktree path.
    assert_eq!(
        source_workspace(&app, project_id, clean_id).await?,
        clean_path.to_string_lossy()
    );

    let (status, cleanup_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{clean_id}/worktree/discard"),
        Some(json!({})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{cleanup_response}");
    assert_eq!(str_field(data(&cleanup_response)?, "status")?, "archived");

    let conflict = create_session(
        &app,
        project_id,
        "conflict continue",
        &base,
        Some("isolated"),
    )
    .await?;
    let conflict_id = str_field(&conflict, "id")?;
    let conflict_worktree = prepare_worktree(&app, conflict_id).await?;
    let conflict_path = PathBuf::from(str_field(&conflict_worktree, "worktree_path")?);
    fs::write(conflict_path.join("conflict.txt"), "session side\n")?;
    git(&conflict_path, &["add", "conflict.txt"])?;
    git(
        &conflict_path,
        &["commit", "-m", "smoke session conflict change"],
    )?;
    fs::write(base.join("conflict.txt"), "current side\n")?;
    git(&base, &["add", "conflict.txt"])?;
    git(&base, &["commit", "-m", "base conflict change"])?;

    let (status, conflict_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{conflict_id}/worktree/merge"),
        Some(json!({ "commit_message": "smoke conflict merge" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{conflict_response}");
    let conflict_data = data(&conflict_response)?;
    assert_eq!(
        conflict_data.get("has_conflicts").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        str_field(conflict_data.get("worktree").unwrap(), "status")?,
        "needs_conflict_resolution"
    );
    assert!(conflict_path.exists());

    let (status, list_response) = api(
        &app,
        Method::GET,
        format!("/api/chat/sessions/{conflict_id}/worktree/merge-conflicts"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{list_response}");
    let files = data(&list_response)?.as_array().unwrap();
    assert!(
        files
            .iter()
            .any(|file| { file.get("path").and_then(Value::as_str) == Some("conflict.txt") })
    );

    let (status, detail_response) = api(
        &app,
        Method::GET,
        format!("/api/chat/sessions/{conflict_id}/worktree/merge-conflicts/conflict.txt"),
        None,
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{detail_response}");
    assert_eq!(str_field(data(&detail_response)?, "path")?, "conflict.txt");

    let (status, resolve_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{conflict_id}/worktree/merge-conflicts/resolve"),
        Some(json!({ "path": "conflict.txt", "content": "resolved smoke\n" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{resolve_response}");
    let (status, continue_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{conflict_id}/worktree/merge/continue"),
        Some(json!({ "commit_message": "smoke conflict resolved" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{continue_response}");
    assert_eq!(str_field(data(&continue_response)?, "status")?, "merged");
    assert_eq!(
        fs::read_to_string(base.join("conflict.txt"))?,
        "resolved smoke\n"
    );
    assert_eq!(
        git(&base, &["log", "-1", "--format=%s"])?,
        "smoke conflict resolved"
    );
    assert!(!git(&base, &["log", "--format=%s"])?.contains("Merge OpenTeams session changes"));
    assert_eq!(
        git(&base, &["rev-list", "--parents", "-n", "1", "HEAD"])?
            .split_whitespace()
            .count(),
        2,
        "conflict resolution must not create a merge commit"
    );

    let aborting =
        create_session(&app, project_id, "conflict abort", &base, Some("isolated")).await?;
    let abort_id = str_field(&aborting, "id")?;
    let abort_worktree = prepare_worktree(&app, abort_id).await?;
    let abort_path = PathBuf::from(str_field(&abort_worktree, "worktree_path")?);
    fs::write(abort_path.join("conflict.txt"), "abort session side\n")?;
    git(&abort_path, &["add", "conflict.txt"])?;
    git(
        &abort_path,
        &["commit", "-m", "smoke abort session conflict change"],
    )?;
    fs::write(base.join("conflict.txt"), "abort current side\n")?;
    git(&base, &["add", "conflict.txt"])?;
    git(&base, &["commit", "-m", "base abort conflict change"])?;
    let (status, abort_merge_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{abort_id}/worktree/merge"),
        Some(json!({ "commit_message": "smoke abort merge" })),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{abort_merge_response}");
    assert_eq!(
        data(&abort_merge_response)?
            .get("has_conflicts")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(abort_path.exists());
    let (status, abort_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{abort_id}/worktree/merge/abort"),
        Some(json!({})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{abort_response}");
    assert!(abort_path.exists());
    let abort_status = str_field(data(&abort_response)?, "status")?;
    assert!(matches!(abort_status, "active" | "dirty"));
    let (status, discard_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{abort_id}/worktree/discard"),
        Some(json!({})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{discard_response}");
    assert_eq!(str_field(data(&discard_response)?, "status")?, "archived");

    let dirty = create_session(&app, project_id, "dirty cleanup", &base, Some("isolated")).await?;
    let dirty_id = str_field(&dirty, "id")?;
    let dirty_worktree = prepare_worktree(&app, dirty_id).await?;
    let dirty_path = PathBuf::from(str_field(&dirty_worktree, "worktree_path")?);
    fs::write(dirty_path.join("dirty.txt"), "dirty unmerged\n")?;
    let (cleanup_status, _) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{dirty_id}/worktree/cleanup"),
        Some(json!({})),
    )
    .await?;
    assert_ne!(cleanup_status, StatusCode::OK);
    let (status, discard_response) = api(
        &app,
        Method::POST,
        format!("/api/chat/sessions/{dirty_id}/worktree/discard"),
        Some(json!({})),
    )
    .await?;
    assert_eq!(status, StatusCode::OK, "{discard_response}");
    assert_eq!(str_field(data(&discard_response)?, "status")?, "archived");

    Ok(())
}

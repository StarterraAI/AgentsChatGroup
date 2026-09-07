use std::{
    path::{Path, PathBuf},
    process::Command,
};

use db::models::chat_session_worktree::{
    CreateSessionWorktree, SessionWorktree, SessionWorktreeMergeOperation, SessionWorktreeMode,
    SessionWorktreeStatus,
};
use sqlx::SqlitePool;
use uuid::Uuid;

use super::{
    EnsureOutcome, EnsureWorktreeInput, MAX_CONFLICT_TEXT_BYTES, SessionWorktreeError,
    SessionWorktreeService, branch_name_for_session, conflict_content_part, detect_git_repo_path,
    is_safe_for_auto_cleanup, parse_unmerged_porcelain, short_session_id, validate_conflict_path,
    validate_transition, worktree_path_for_session,
};

fn git(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout={}\nstderr={}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_fails(repo: &Path, args: &[&str]) -> bool {
    !Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git")
        .status
        .success()
}

fn init_git_repo(repo: &Path) {
    std::fs::create_dir_all(repo).expect("create repo dir");
    let output = Command::new("git")
        .arg("init")
        .arg("-b")
        .arg("main")
        .arg(repo)
        .output()
        .expect("git init");
    assert!(
        output.status.success(),
        "git init failed\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    git(
        repo,
        &["config", "user.email", "session-worktree@example.test"],
    );
    git(repo, &["config", "user.name", "Session Worktree Test"]);
    std::fs::write(repo.join("README.md"), "base\n").expect("write seed file");
    git(repo, &["add", "."]);
    git(repo, &["commit", "-m", "initial"]);
}

struct TestWorktreeCleanup {
    path: PathBuf,
}

impl Drop for TestWorktreeCleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

async fn ensure_test_worktree(
    service: &SessionWorktreeService,
    session_id: Uuid,
    base: PathBuf,
) -> (SessionWorktree, TestWorktreeCleanup) {
    let worktree = match service
        .ensure_for_session(EnsureWorktreeInput::new(session_id, base))
        .await
        .expect("create session worktree")
    {
        EnsureOutcome::Created(row) | EnsureOutcome::Existing(row) => row,
    };
    let cleanup = TestWorktreeCleanup {
        path: PathBuf::from(&worktree.worktree_path),
    };
    (worktree, cleanup)
}

#[tokio::test]
async fn session_deletion_force_cleanup_removes_active_worktree_and_branch() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool);
    let session_id = Uuid::new_v4();
    let temp = tempfile::tempdir().expect("create temp dir");
    let base = temp.path().join("repo");
    init_git_repo(&base);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;
    let worktree_path = PathBuf::from(&worktree.worktree_path);
    assert!(worktree_path.exists());
    assert!(!git_fails(
        &base,
        &[
            "show-ref",
            "--verify",
            &format!("refs/heads/{}", worktree.branch_name)
        ]
    ));

    service
        .force_cleanup_for_session_deletion(session_id)
        .await
        .expect("force cleanup before session deletion");

    assert!(!worktree_path.exists());
    assert!(git_fails(
        &base,
        &[
            "show-ref",
            "--verify",
            &format!("refs/heads/{}", worktree.branch_name)
        ]
    ));
}

#[tokio::test]
async fn session_deletion_force_cleanup_is_a_noop_without_worktree() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool);

    service
        .force_cleanup_for_session_deletion(Uuid::new_v4())
        .await
        .expect("sessions without isolated worktrees need no cleanup");
}

/// In-memory schema matching `20260622120000_create_chat_session_worktrees.sql`.
/// Kept in sync by hand; tests fail loudly if the model and schema diverge.
async fn setup_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:")
        .await
        .expect("create sqlite memory pool");
    sqlx::query(
        r#"
        CREATE TABLE chat_session_worktrees (
            id                    BLOB    NOT NULL PRIMARY KEY,
            session_id            BLOB    NOT NULL,
            project_id            BLOB,
            base_workspace_path   TEXT    NOT NULL,
            repo_path             TEXT    NOT NULL,
            base_branch           TEXT    NOT NULL,
            base_commit           TEXT,
            branch_name           TEXT    NOT NULL,
            worktree_path         TEXT    NOT NULL,
            mode                  TEXT    NOT NULL DEFAULT 'session'
                                        CHECK (mode IN ('session')),
            status                TEXT    NOT NULL DEFAULT 'creating'
                                        CHECK (status IN (
                                            'creating', 'active', 'dirty', 'merging',
                                            'needs_conflict_resolution', 'merged',
                                            'archived', 'cleanup_pending', 'cleanup_failed'
                                        )),
            merge_target_branch   TEXT,
            merge_operation       TEXT
                                        CHECK (merge_operation IS NULL
                                               OR merge_operation IN (
                                                   'merge', 'squash_merge', 'cherry_pick', 'rebase'
                                               )),
            conflict_files_json   TEXT    NOT NULL DEFAULT '[]',
            operation_started_at  TEXT,
            cleanup_error         TEXT,
            last_used_at          TEXT,
            merged_at             TEXT,
            archived_at           TEXT,
            created_at            TEXT    NOT NULL DEFAULT (datetime('now', 'subsec')),
            updated_at            TEXT    NOT NULL DEFAULT (datetime('now', 'subsec'))
        );

        CREATE UNIQUE INDEX idx_chat_session_worktrees_active_session
            ON chat_session_worktrees(session_id)
            WHERE status IN ('creating', 'active', 'dirty', 'merging',
                             'needs_conflict_resolution', 'merged', 'cleanup_pending');

        CREATE TABLE chat_session_agents (
            id                  BLOB    NOT NULL PRIMARY KEY,
            session_id          BLOB    NOT NULL,
            agent_id            BLOB    NOT NULL,
            state               TEXT    NOT NULL DEFAULT 'idle',
            workspace_path      TEXT,
            pty_session_key     TEXT,
            agent_session_id    TEXT,
            agent_message_id    BLOB,
            project_member_id   BLOB,
            member_name         TEXT    NOT NULL DEFAULT '',
            execution_config    TEXT    NOT NULL DEFAULT '{}',
            allowed_skill_ids   TEXT    NOT NULL DEFAULT '[]',
            created_at          TEXT    NOT NULL DEFAULT (datetime('now', 'subsec')),
            updated_at          TEXT    NOT NULL DEFAULT (datetime('now', 'subsec'))
        );
        "#,
    )
    .execute(&pool)
    .await
    .expect("create chat_session_worktrees test schema");
    pool
}

/// Seed a row directly into `status`, bypassing the reducer. The reducer
/// itself is covered by the service-level tests further down; here we only
/// need a row visible to the model and the safety guards.
async fn seed_row(
    pool: &SqlitePool,
    session_id: Uuid,
    status: SessionWorktreeStatus,
) -> SessionWorktree {
    seed_row_with_base_path(pool, session_id, status, "/tmp/base").await
}

async fn seed_row_with_base_path(
    pool: &SqlitePool,
    session_id: Uuid,
    status: SessionWorktreeStatus,
    base_workspace_path: impl Into<String>,
) -> SessionWorktree {
    let id = Uuid::new_v4();
    let base_workspace_path = base_workspace_path.into();
    let row = SessionWorktree::create(
        pool,
        &CreateSessionWorktree {
            session_id,
            project_id: None,
            base_workspace_path: base_workspace_path.clone(),
            repo_path: base_workspace_path,
            base_branch: "main".to_string(),
            base_commit: None,
            branch_name: format!("openteams/session/{}", short_session_id(session_id)),
            worktree_path: format!("/tmp/wt/{}", short_session_id(session_id)),
            mode: SessionWorktreeMode::Session,
        },
        id,
    )
    .await
    .expect("seed row");

    sqlx::query(
        r#"
        UPDATE chat_session_worktrees
        SET status = ?2,
            updated_at = datetime('now', 'subsec')
        WHERE id = ?1
        "#,
    )
    .bind(row.id)
    .bind(status)
    .execute(pool)
    .await
    .expect("force status");

    SessionWorktree::find_by_id(pool, row.id)
        .await
        .expect("re-read")
        .expect("row present")
}

// ---------------------------------------------------------------------------
// Pure helper tests
// ---------------------------------------------------------------------------

#[test]
fn short_session_id_is_first_eight_hex_chars_lowercased() {
    let id = Uuid::parse_str("AbCdEf01-2345-6789-0123-456789abcdef").unwrap();
    let short = short_session_id(id);
    assert_eq!(short.len(), 8);
    assert_eq!(short, "abcdef01");
}

#[test]
fn branch_name_for_session_uses_stable_prefix_and_short_id() {
    let id = Uuid::parse_str("12345678-4455-6789-0123-456789abcdef").unwrap();
    let branch = branch_name_for_session(id);
    assert_eq!(branch, "openteams/session/12345678");

    // Idempotent: same session -> same branch.
    assert_eq!(branch, branch_name_for_session(id));

    // Different session -> different branch.
    let other = Uuid::parse_str("abcdef01-4455-6789-0123-456789abcdef").unwrap();
    assert_ne!(branch_name_for_session(other), branch);
}

#[test]
fn worktree_path_lives_under_sessions_subdir() {
    let id = Uuid::new_v4();
    let path = worktree_path_for_session(id);
    let last_two: Vec<_> = path.components().rev().take(2).collect();
    let sessions_component = last_two[1].as_os_str().to_string_lossy();
    let short_component = last_two[0].as_os_str().to_string_lossy();
    assert_eq!(sessions_component, "sessions");
    assert_eq!(short_component, short_session_id(id));
}

#[test]
fn detect_git_repo_path_walks_up_to_find_dot_git() {
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".git")).unwrap();
    std::fs::create_dir_all(root.join("nested/sub/dir")).unwrap();

    let found = detect_git_repo_path(&root.join("nested/sub/dir")).unwrap();
    assert_eq!(found, root);

    // A directory without .git anywhere up the tree returns None.
    let outside = tempfile::TempDir::new().unwrap();
    let outside_root = outside.path().canonicalize().unwrap();
    assert_eq!(detect_git_repo_path(&outside_root), None);
}

// ---------------------------------------------------------------------------
// State machine tests
// ---------------------------------------------------------------------------

#[test]
fn validate_transition_accepts_documented_legal_paths() {
    use SessionWorktreeStatus::*;
    let legal = [
        (Creating, Active),
        (Creating, CleanupFailed),
        (Creating, Archived),
        (Active, Dirty),
        (Active, Merging),
        (Active, CleanupPending),
        (Active, Archived),
        (Active, CleanupFailed),
        (Dirty, Active),
        (Dirty, Merging),
        (Dirty, CleanupPending),
        (Dirty, Archived),
        (Dirty, CleanupFailed),
        (Merging, Merged),
        (Merging, NeedsConflictResolution),
        (Merging, Dirty),
        (Merging, Active),
        (Merging, CleanupPending),
        (Merging, CleanupFailed),
        (NeedsConflictResolution, Merged),
        (NeedsConflictResolution, Dirty),
        (NeedsConflictResolution, Merging),
        (NeedsConflictResolution, CleanupPending),
        (NeedsConflictResolution, CleanupFailed),
        (Merged, Dirty),
        (Merged, CleanupPending),
        (Merged, Archived),
        (CleanupPending, Archived),
        (CleanupPending, CleanupFailed),
        (CleanupFailed, CleanupPending),
        (CleanupFailed, Archived),
    ];
    for (from, to) in legal {
        assert!(
            validate_transition(from, to).is_ok(),
            "expected {from:?} -> {to:?} to be legal"
        );
    }
}

#[test]
fn validate_transition_rejects_illegal_paths() {
    use SessionWorktreeStatus::*;
    let illegal = [
        (Creating, Dirty),
        (Creating, Merging),
        (Creating, Merged),
        (Active, NeedsConflictResolution),
        (Active, Merged),
        (Dirty, Merged),
        (Dirty, NeedsConflictResolution),
        (Merging, Creating),
        (NeedsConflictResolution, Active),
        (NeedsConflictResolution, Creating),
        (Merged, Active),
        (Merged, Merging),
        (CleanupPending, Active),
        (CleanupPending, Merging),
        (CleanupFailed, Active),
        (CleanupFailed, Merging),
        (Archived, Active),
        (Archived, Merging),
        (Archived, CleanupPending),
        (Archived, Merged),
    ];
    for (from, to) in illegal {
        assert!(
            validate_transition(from, to).is_err(),
            "expected {from:?} -> {to:?} to be rejected"
        );
    }
}

#[test]
fn validate_transition_accepts_self_transitions_as_idempotent() {
    use SessionWorktreeStatus::*;
    for status in [
        Creating,
        Active,
        Dirty,
        Merging,
        NeedsConflictResolution,
        Merged,
        Archived,
        CleanupPending,
        CleanupFailed,
    ] {
        assert!(validate_transition(status, status).is_ok());
    }
}

#[test]
fn display_matches_serde_wire_format_for_all_enums() {
    // Critical wire-format invariant (AGENTS.md workflow pitfall): the typed
    // `Display` impl must produce the exact `snake_case` string that serde
    // emits, so callers can use `format!("{}", status)` instead of the
    // forbidden `format!("{:?}", status).to_lowercase()`.
    for status in [
        SessionWorktreeStatus::Creating,
        SessionWorktreeStatus::Active,
        SessionWorktreeStatus::Dirty,
        SessionWorktreeStatus::Merging,
        SessionWorktreeStatus::NeedsConflictResolution,
        SessionWorktreeStatus::Merged,
        SessionWorktreeStatus::Archived,
        SessionWorktreeStatus::CleanupPending,
        SessionWorktreeStatus::CleanupFailed,
    ] {
        let display = format!("{status}");
        let serde_str = serde_json::to_value(status)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(
            display, serde_str,
            "Display and serde wire format diverge for {status:?}"
        );
    }

    let mode = SessionWorktreeMode::Session;
    let display = format!("{mode}");
    let serde_str = serde_json::to_value(mode)
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(display, serde_str);

    for op in [
        SessionWorktreeMergeOperation::Merge,
        SessionWorktreeMergeOperation::SquashMerge,
        SessionWorktreeMergeOperation::CherryPick,
        SessionWorktreeMergeOperation::Rebase,
    ] {
        let display = format!("{op}");
        let serde_str = serde_json::to_value(op)
            .unwrap()
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(display, serde_str);
    }

    // Spot-check the exact expected strings so a future variant rename is
    // caught at the test level even if serde attribute is missed.
    assert_eq!(
        format!("{}", SessionWorktreeStatus::NeedsConflictResolution),
        "needs_conflict_resolution"
    );
    assert_eq!(
        format!("{}", SessionWorktreeStatus::CleanupPending),
        "cleanup_pending"
    );
    assert_eq!(
        format!("{}", SessionWorktreeStatus::CleanupFailed),
        "cleanup_failed"
    );
    assert_eq!(format!("{}", SessionWorktreeMergeOperation::Merge), "merge");
    assert_eq!(
        format!("{}", SessionWorktreeMergeOperation::SquashMerge),
        "squash_merge"
    );
    assert_eq!(
        format!("{}", SessionWorktreeMergeOperation::CherryPick),
        "cherry_pick"
    );
}

#[test]
fn is_safe_for_auto_cleanup_blocks_unmerged_runtime_states() {
    use SessionWorktreeStatus::*;
    // Critical safety invariant: active runtime states and merged worktrees
    // must never be force-removed by an automatic janitor pass. Only an
    // explicit user `discard` may move them to `cleanup_pending`.
    for blocked in [
        Creating,
        Active,
        Dirty,
        Merging,
        NeedsConflictResolution,
        Merged,
    ] {
        assert!(
            !is_safe_for_auto_cleanup(blocked),
            "{blocked:?} must never be auto-cleaned"
        );
    }
    for safe in [CleanupPending, CleanupFailed, Archived] {
        assert!(is_safe_for_auto_cleanup(safe), "{safe:?} should be safe");
    }
}

#[test]
fn is_active_for_workspace_keeps_merged_worktrees_active_until_discarded() {
    use SessionWorktreeStatus::*;
    // These statuses use the worktree path as the active workspace.
    for active in [
        Creating,
        Active,
        Dirty,
        Merging,
        NeedsConflictResolution,
        Merged,
    ] {
        assert!(
            active.is_active_for_workspace(),
            "{active:?} should be active for workspace selection"
        );
    }
    // Cleanup and terminal/audit statuses switch back to base_workspace_path.
    for inactive in [CleanupPending, Archived, CleanupFailed] {
        assert!(
            !inactive.is_active_for_workspace(),
            "{inactive:?} should NOT be active for workspace selection"
        );
    }
}

// ---------------------------------------------------------------------------
// DB model tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_active_by_session_excludes_terminal_rows() {
    let pool = setup_pool().await;
    let session_id = Uuid::new_v4();

    // No row yet.
    assert!(
        SessionWorktree::find_active_by_session(&pool, session_id)
            .await
            .unwrap()
            .is_none()
    );

    let merged = seed_row(&pool, session_id, SessionWorktreeStatus::Merged).await;
    let active_lookup = SessionWorktree::find_active_by_session(&pool, session_id)
        .await
        .unwrap()
        .expect("merged row remains available for lifecycle operations");
    assert_eq!(active_lookup.id, merged.id);

    let archived_session_id = Uuid::new_v4();
    seed_row(&pool, archived_session_id, SessionWorktreeStatus::Archived).await;
    assert!(
        SessionWorktree::find_active_by_session(&pool, archived_session_id)
            .await
            .unwrap()
            .is_none()
    );
}

// ---------------------------------------------------------------------------
// Conflict path validation tests (security-critical: prevents path traversal)
// ---------------------------------------------------------------------------

#[test]
fn validate_conflict_path_accepts_simple_relative_paths() {
    assert!(validate_conflict_path("src/main.rs").is_ok());
    assert!(validate_conflict_path("docs/readme.md").is_ok());
    assert!(validate_conflict_path("a/b/c/file.txt").is_ok());
}

#[test]
fn validate_conflict_path_rejects_absolute_paths() {
    assert!(validate_conflict_path("/etc/passwd").is_err());
    assert!(validate_conflict_path("C:/Windows/System32").is_err());
}

#[test]
fn validate_conflict_path_rejects_parent_dir_traversal() {
    assert!(validate_conflict_path("../secret").is_err());
    assert!(validate_conflict_path("a/../../b").is_err());
    assert!(validate_conflict_path("a/b/../../../c").is_err());
}

// ---------------------------------------------------------------------------
// Merge-state guard tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_conflict_files_rejects_session_without_merge_in_progress() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();

    // Seed an active (non-merge) worktree row.
    seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let err = service
        .list_conflict_files(session_id)
        .await
        .expect_err("must reject when no merge in progress");
    assert!(matches!(
        err,
        SessionWorktreeError::NoMergeInProgress(sid) if sid == session_id
    ));
}

#[tokio::test]
async fn list_conflict_files_rejects_session_without_worktree() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();

    let err = service
        .list_conflict_files(session_id)
        .await
        .expect_err("must reject when no worktree exists");
    assert!(matches!(
        err,
        SessionWorktreeError::NoActiveWorktree(sid) if sid == session_id
    ));
}

#[tokio::test]
async fn continue_merge_rejects_when_no_merge_in_progress() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();

    seed_row(&pool, session_id, SessionWorktreeStatus::Dirty).await;

    let err = service
        .continue_merge(session_id, None)
        .await
        .expect_err("must reject when no merge in progress");
    assert!(matches!(
        err,
        SessionWorktreeError::NoMergeInProgress(sid) if sid == session_id
    ));
}

#[tokio::test]
async fn perform_abort_merge_rejects_when_no_merge_in_progress() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();

    seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let err = service
        .perform_abort_merge(session_id)
        .await
        .expect_err("must reject when no merge in progress");
    assert!(matches!(
        err,
        SessionWorktreeError::NoMergeInProgress(sid) if sid == session_id
    ));
}

#[tokio::test]
async fn read_conflict_file_rejects_invalid_path() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();

    // Need a needs_conflict_resolution row to get past the status guard.
    seed_row(
        &pool,
        session_id,
        SessionWorktreeStatus::NeedsConflictResolution,
    )
    .await;

    let err = service
        .read_conflict_file(session_id, "../../../etc/passwd")
        .await
        .expect_err("must reject path traversal");
    assert!(matches!(err, SessionWorktreeError::InvalidConflictPath(_)));
}

#[test]
fn parse_unmerged_porcelain_reports_specific_conflict_statuses() {
    let parsed = parse_unmerged_porcelain(
        "UU src/both.rs\nDU src/deleted-by-us.rs\nUD src/deleted-by-them.rs\nAA src/both-added.rs\nR  src/old.rs -> src/new.rs\n M src/ordinary.rs\n",
    );

    let pairs: Vec<_> = parsed
        .into_iter()
        .map(|info| (info.path, info.status))
        .collect();
    assert_eq!(
        pairs,
        vec![
            ("src/both.rs".to_string(), "both_modified".to_string()),
            (
                "src/deleted-by-us.rs".to_string(),
                "deleted_by_us".to_string()
            ),
            (
                "src/deleted-by-them.rs".to_string(),
                "deleted_by_them".to_string()
            ),
            ("src/both-added.rs".to_string(), "both_added".to_string()),
            ("src/new.rs".to_string(), "renamed".to_string()),
        ]
    );
}

#[tokio::test]
async fn read_conflict_file_marks_binary_without_treating_it_as_text() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(root.join("image.bin"), [0, 159, 146, 150]).unwrap();
    seed_row_with_base_path(
        &pool,
        session_id,
        SessionWorktreeStatus::NeedsConflictResolution,
        root.to_string_lossy().to_string(),
    )
    .await;

    let detail = service
        .read_conflict_file(session_id, "image.bin")
        .await
        .expect("binary detail should load");

    assert!(detail.is_binary);
    assert!(!detail.is_too_large);
    assert_eq!(detail.working_tree, "");
    assert_eq!(detail.size_bytes, 4);
}

#[tokio::test]
async fn read_conflict_file_marks_too_large_text_without_loading_it() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    std::fs::write(
        root.join("large.txt"),
        "a".repeat(MAX_CONFLICT_TEXT_BYTES + 1),
    )
    .unwrap();
    seed_row_with_base_path(
        &pool,
        session_id,
        SessionWorktreeStatus::NeedsConflictResolution,
        root.to_string_lossy().to_string(),
    )
    .await;

    let detail = service
        .read_conflict_file(session_id, "large.txt")
        .await
        .expect("large detail should load");

    assert!(!detail.is_binary);
    assert!(detail.is_too_large);
    assert_eq!(detail.working_tree, "");
    assert_eq!(detail.size_bytes, (MAX_CONFLICT_TEXT_BYTES + 1) as u64);

    let small = conflict_content_part(Some(b"small text"));
    assert_eq!(small.text.as_deref(), Some("small text"));
    assert!(!small.is_binary);
    assert!(!small.is_too_large);
}

#[tokio::test]
async fn transition_status_rejects_cas_when_status_differs() {
    let pool = setup_pool().await;
    let session_id = Uuid::new_v4();
    let row = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    // CAS expects Creating but the row is Active — must fail.
    let err = SessionWorktree::transition_status(
        &pool,
        row.id,
        SessionWorktreeStatus::Creating,
        SessionWorktreeStatus::Active,
    )
    .await
    .expect_err("CAS must reject when expected from does not match");
    assert!(matches!(
        err,
        db::models::chat_session_worktree::SessionWorktreeError::CasRejected { .. }
    ));
}

#[tokio::test]
async fn transition_status_is_atomic_for_legal_pair() {
    let pool = setup_pool().await;
    let session_id = Uuid::new_v4();
    let row = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let updated = SessionWorktree::transition_status(
        &pool,
        row.id,
        SessionWorktreeStatus::Active,
        SessionWorktreeStatus::Merging,
    )
    .await
    .expect("CAS legal transition");
    assert_eq!(updated.status, SessionWorktreeStatus::Merging);
    assert_eq!(updated.id, row.id);
}

#[tokio::test]
async fn set_conflict_files_sorts_and_dedups() {
    let pool = setup_pool().await;
    let session_id = Uuid::new_v4();
    let row = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let updated = SessionWorktree::set_conflict_files(
        &pool,
        row.id,
        &[
            "b.txt".to_string(),
            "a.txt".to_string(),
            "".to_string(),
            "b.txt".to_string(),
        ],
    )
    .await
    .unwrap();
    assert_eq!(updated.conflict_files(), vec!["a.txt", "b.txt"]);
}

#[tokio::test]
async fn conflict_files_parses_to_empty_on_corrupt_json() {
    let pool = setup_pool().await;
    let session_id = Uuid::new_v4();
    let row = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;
    sqlx::query("UPDATE chat_session_worktrees SET conflict_files_json = 'not-json' WHERE id = ?")
        .bind(row.id)
        .execute(&pool)
        .await
        .unwrap();
    let reloaded = SessionWorktree::find_by_id(&pool, row.id)
        .await
        .unwrap()
        .unwrap();
    assert!(reloaded.conflict_files().is_empty());
}

#[tokio::test]
async fn find_latest_by_session_and_status_returns_newest_match() {
    let pool = setup_pool().await;
    let session_id = Uuid::new_v4();

    // Two historical `archived` rows plus one `active` row for the same
    // session. The active lookup must not see either archived row; the
    // targeted lookup must return the newest archived row.
    let first_archived = seed_row(&pool, session_id, SessionWorktreeStatus::Archived).await;
    // Tiny sleep so created_at differs and ORDER BY is deterministic.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    let second_archived = seed_row(&pool, session_id, SessionWorktreeStatus::Archived).await;
    let _active = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    // find_active_by_session must return the active row, not the archived ones.
    let active_lookup = SessionWorktree::find_active_by_session(&pool, session_id)
        .await
        .unwrap()
        .expect("active row visible");
    assert_eq!(active_lookup.status, SessionWorktreeStatus::Active);

    // Targeted lookup returns the LATEST archived row (second_archived).
    let archived_lookup = SessionWorktree::find_latest_by_session_and_status(
        &pool,
        session_id,
        SessionWorktreeStatus::Archived,
    )
    .await
    .unwrap()
    .expect("archived row visible via targeted lookup");
    assert_eq!(archived_lookup.id, second_archived.id);
    assert_ne!(archived_lookup.id, first_archived.id);

    // Targeted lookup for a status with no row returns None.
    assert!(
        SessionWorktree::find_latest_by_session_and_status(
            &pool,
            session_id,
            SessionWorktreeStatus::CleanupFailed
        )
        .await
        .unwrap()
        .is_none()
    );
}

// ---------------------------------------------------------------------------
// Service reducer tests
//
// Reducer tests use a service constructed over the in-memory pool. They
// exercise the validate-then-CAS path and the safety guards. The discard /
// cleanup paths call `WorktreeManager::cleanup_worktree` directly and need
// a real git repo; those flows are validated by the worktree_manager's own
// tests, so we do not duplicate them here.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn service_merge_records_intent_only_in_skeleton() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let row = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let merging = service
        .merge_session_changes(
            session_id,
            SessionWorktreeMergeOperation::Merge,
            Some("main".to_string()),
        )
        .await
        .expect("merge skeleton transitions to merging");

    assert_eq!(merging.status, SessionWorktreeStatus::Merging);
    assert_eq!(merging.id, row.id);
    let reloaded = SessionWorktree::find_by_id(&pool, row.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        reloaded.merge_operation,
        Some(SessionWorktreeMergeOperation::Merge)
    );
    assert_eq!(reloaded.merge_target_branch.as_deref(), Some("main"));
    assert!(reloaded.operation_started_at.is_some());
}

#[tokio::test]
async fn service_mark_merged_keeps_session_agent_workspace_paths() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let merging = seed_row(&pool, session_id, SessionWorktreeStatus::Merging).await;
    let session_agent_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO chat_session_agents (
            id, session_id, agent_id, state, workspace_path
        )
        VALUES (?1, ?2, ?3, 'idle', ?4)
        "#,
    )
    .bind(session_agent_id)
    .bind(session_id)
    .bind(Uuid::new_v4())
    .bind(&merging.worktree_path)
    .execute(&pool)
    .await
    .expect("insert session agent");

    let merged = service.mark_merged(session_id).await.expect("mark merged");
    assert_eq!(merged.status, SessionWorktreeStatus::Merged);

    let workspace_path: String =
        sqlx::query_scalar("SELECT workspace_path FROM chat_session_agents WHERE id = ?1")
            .bind(session_agent_id)
            .fetch_one(&pool)
            .await
            .expect("read retained workspace path");
    assert_eq!(workspace_path, merging.worktree_path);
}

#[tokio::test]
async fn ensure_for_session_uses_current_workspace_branch_over_origin_head() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);

    git(&base, &["update-ref", "refs/remotes/origin/main", "main"]);
    git(
        &base,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/main",
        ],
    );
    git(&base, &["checkout", "-b", "codex/openteams-dev"]);
    std::fs::write(base.join("branch.txt"), "current branch only\n").expect("write branch file");
    git(&base, &["add", "branch.txt"]);
    git(&base, &["commit", "-m", "current branch commit"]);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;

    assert_eq!(worktree.base_branch, "codex/openteams-dev");
    let expected_head = git(&base, &["rev-parse", "codex/openteams-dev"]);
    assert_eq!(
        worktree.base_commit.as_deref(),
        Some(expected_head.as_str())
    );
    assert!(
        PathBuf::from(&worktree.worktree_path)
            .join("branch.txt")
            .exists(),
        "session worktree should be forked from the current workspace branch"
    );
}

#[tokio::test]
async fn cherry_pick_merge_preserves_session_commit_without_merge_commit() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;

    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("feature.txt"), "from session\n")
        .expect("write worktree change");
    git(&worktree_path, &["add", "feature.txt"]);
    git(&worktree_path, &["commit", "-m", "session branch commit"]);

    let merge_available = service
        .get_latest_for_session(session_id)
        .await
        .expect("read worktree merge availability")
        .expect("worktree row");
    assert!(merge_available.has_unmerged_commits);

    service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::CherryPick,
            None,
            None,
        )
        .await
        .expect("merge worktree");

    let head_parents = git(&base, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        head_parents.split_whitespace().count(),
        2,
        "integration should create a normal one-parent commit"
    );
    assert_eq!(
        git(&base, &["log", "-1", "--format=%s"]),
        "session branch commit"
    );
    assert!(!git(&base, &["log", "--format=%s"]).contains("Merge OpenTeams session changes"));

    std::fs::write(worktree_path.join("second.txt"), "second session commit\n")
        .expect("write second worktree change");
    git(&worktree_path, &["add", "."]);
    git(&worktree_path, &["commit", "-m", "second session commit"]);

    // After merge, new commits on the session branch transition the worktree
    // back to `Dirty` so the user can re-merge or discard.
    let refreshed = service
        .get_latest_for_session(session_id)
        .await
        .expect("refresh merged worktree")
        .expect("worktree row");
    assert_eq!(refreshed.status, SessionWorktreeStatus::Dirty);
    assert!(refreshed.has_unmerged_commits);

    service
        .discard_worktree(session_id)
        .await
        .expect("cleanup dirty worktree");
    assert!(
        git_fails(&base, &["rev-parse", "--verify", &worktree.branch_name]),
        "discard should delete the session worktree branch"
    );
}

#[tokio::test]
async fn cherry_pick_conflict_resolution_does_not_create_merge_commit() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;
    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("README.md"), "from session\n")
        .expect("write session conflict");
    git(&worktree_path, &["add", "README.md"]);
    git(&worktree_path, &["commit", "-m", "session conflict change"]);

    std::fs::write(base.join("README.md"), "from base\n").expect("write base conflict");
    git(&base, &["add", "README.md"]);
    git(&base, &["commit", "-m", "base conflict change"]);

    let result = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::CherryPick,
            None,
            None,
        )
        .await
        .expect("start cherry-pick");
    assert!(result.has_conflicts);

    service
        .resolve_conflict_file(
            session_id,
            "README.md",
            Some("resolved content\n"),
            None,
            false,
        )
        .await
        .expect("resolve conflict");
    let merged = service
        .continue_merge(session_id, None)
        .await
        .expect("continue cherry-pick");

    assert_eq!(merged.status, SessionWorktreeStatus::Merged);
    assert_eq!(
        git(&base, &["log", "-1", "--format=%s"]),
        "session conflict change"
    );
    let head_parents = git(&base, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(head_parents.split_whitespace().count(), 2);
    assert!(!git(&base, &["log", "--format=%s"]).contains("Merge OpenTeams session changes"));
}

#[tokio::test]
async fn squash_merge_combines_session_commits_and_tracks_incremental_cursor() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);
    std::fs::write(base.join("base-only.txt"), "committed base\n").expect("write base-only file");
    git(&base, &["add", "base-only.txt"]);
    git(&base, &["commit", "-m", "add base-only file"]);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;
    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("first.txt"), "first\n").expect("write first change");
    git(&worktree_path, &["add", "first.txt"]);
    git(&worktree_path, &["commit", "-m", "first session commit"]);
    std::fs::write(worktree_path.join("second.txt"), "second\n").expect("write second change");
    git(&worktree_path, &["add", "second.txt"]);
    git(&worktree_path, &["commit", "-m", "second session commit"]);
    std::fs::write(base.join("base-only.txt"), "local base edit\n")
        .expect("write unrelated local edit");

    let first_result = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::SquashMerge,
            None,
            Some("squash session changes".to_string()),
        )
        .await
        .expect("squash session commits");

    assert!(!first_result.has_conflicts);
    assert_eq!(first_result.worktree.status, SessionWorktreeStatus::Merged);
    assert!(base.join("first.txt").exists());
    assert!(base.join("second.txt").exists());
    assert_eq!(
        std::fs::read_to_string(base.join("base-only.txt")).expect("read local base edit"),
        "local base edit\n"
    );
    assert_eq!(
        git(&base, &["show", "HEAD:base-only.txt"]),
        "committed base",
        "squash commit must exclude pre-existing base edits"
    );
    assert!(
        git(&base, &["status", "--porcelain", "--untracked-files=no"]).contains("M base-only.txt")
    );
    assert_eq!(
        git(&base, &["log", "-1", "--format=%s"]),
        "squash session changes"
    );
    let head_parents = git(&base, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(
        head_parents.split_whitespace().count(),
        2,
        "squash integration must create a normal one-parent commit"
    );
    let base_subjects = git(&base, &["log", "--format=%s"]);
    assert!(!base_subjects.contains("first session commit"));
    assert!(!base_subjects.contains("second session commit"));

    std::fs::write(worktree_path.join("third.txt"), "third\n").expect("write third change");
    git(&worktree_path, &["add", "third.txt"]);
    git(&worktree_path, &["commit", "-m", "third session commit"]);

    let dirty = service
        .get_latest_for_session(session_id)
        .await
        .expect("refresh squash availability")
        .expect("worktree row");
    assert_eq!(dirty.status, SessionWorktreeStatus::Dirty);
    assert!(dirty.has_unmerged_commits);

    let second_result = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::SquashMerge,
            None,
            Some("second squash".to_string()),
        )
        .await
        .expect("squash only new session changes");
    assert!(!second_result.has_conflicts);
    assert!(base.join("third.txt").exists());
    assert_eq!(git(&base, &["log", "-1", "--format=%s"]), "second squash");

    let merged = service
        .get_latest_for_session(session_id)
        .await
        .expect("refresh merged squash")
        .expect("worktree row");
    assert_eq!(merged.status, SessionWorktreeStatus::Merged);
    assert!(!merged.has_unmerged_commits);
}

#[tokio::test]
async fn squash_merge_resolves_multi_commit_conflict_once_without_replaying_it() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;
    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("README.md"), "session one\n")
        .expect("write first session conflict");
    git(&worktree_path, &["add", "README.md"]);
    git(&worktree_path, &["commit", "-m", "first session conflict"]);
    std::fs::write(worktree_path.join("README.md"), "session two\n")
        .expect("write second session conflict");
    git(&worktree_path, &["add", "README.md"]);
    git(&worktree_path, &["commit", "-m", "second session conflict"]);

    std::fs::write(base.join("README.md"), "base divergent\n").expect("write base conflict");
    git(&base, &["add", "README.md"]);
    git(&base, &["commit", "-m", "base divergent change"]);

    let result = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::SquashMerge,
            None,
            Some("squash conflicting changes".to_string()),
        )
        .await
        .expect("start squash conflict");
    assert!(result.has_conflicts);
    assert_eq!(result.conflict_files, vec!["README.md"]);

    service
        .resolve_conflict_file(
            session_id,
            "README.md",
            Some("resolved once\n"),
            None,
            false,
        )
        .await
        .expect("resolve combined conflict");
    let merged = service
        .continue_merge(session_id, Some("squash conflicting changes".to_string()))
        .await
        .expect("finish squash conflict");

    assert_eq!(merged.status, SessionWorktreeStatus::Merged);
    assert!(merged.conflict_files().is_empty());
    assert_eq!(
        std::fs::read_to_string(base.join("README.md")).expect("read resolved file"),
        "resolved once\n"
    );
    assert!(git(&base, &["diff", "--name-only", "--diff-filter=U"]).is_empty());
    let head_parents = git(&base, &["rev-list", "--parents", "-n", "1", "HEAD"]);
    assert_eq!(head_parents.split_whitespace().count(), 2);

    // A later session commit that does not touch README must integrate from
    // the recorded cursor instead of replaying the already-resolved change.
    std::fs::write(worktree_path.join("later.txt"), "later\n").expect("write later change");
    git(&worktree_path, &["add", "later.txt"]);
    git(&worktree_path, &["commit", "-m", "later session commit"]);
    let dirty = service
        .get_latest_for_session(session_id)
        .await
        .expect("refresh dirty worktree")
        .expect("worktree row");
    assert_eq!(dirty.status, SessionWorktreeStatus::Dirty);

    let repeated = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::SquashMerge,
            None,
            Some("later squash".to_string()),
        )
        .await
        .expect("merge later change without replaying old conflict");
    assert!(!repeated.has_conflicts);
    assert!(base.join("later.txt").exists());
    assert_eq!(
        std::fs::read_to_string(base.join("README.md")).expect("read retained resolution"),
        "resolved once\n"
    );
}

#[tokio::test]
async fn abort_squash_conflict_preserves_unrelated_base_edits() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);
    std::fs::write(base.join("base-only.txt"), "committed base\n").expect("write base-only file");
    git(&base, &["add", "base-only.txt"]);
    git(&base, &["commit", "-m", "add base-only file"]);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;
    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("README.md"), "session side\n")
        .expect("write session conflict");
    git(&worktree_path, &["add", "README.md"]);
    git(&worktree_path, &["commit", "-m", "session conflict"]);

    std::fs::write(base.join("README.md"), "base side\n").expect("write base conflict");
    git(&base, &["add", "README.md"]);
    git(&base, &["commit", "-m", "base conflict"]);
    std::fs::write(base.join("base-only.txt"), "local base edit\n")
        .expect("write unrelated local edit");

    let result = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::SquashMerge,
            None,
            Some("aborted squash".to_string()),
        )
        .await
        .expect("start squash conflict");
    assert!(result.has_conflicts);

    let aborted = service
        .perform_abort_merge(session_id)
        .await
        .expect("abort squash conflict");
    assert_eq!(aborted.status, SessionWorktreeStatus::Dirty);
    assert_eq!(
        std::fs::read_to_string(base.join("README.md")).expect("read restored base file"),
        "base side\n"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("base-only.txt")).expect("read local base edit"),
        "local base edit\n"
    );
    assert!(git(&base, &["diff", "--name-only", "--diff-filter=U"]).is_empty());
    assert!(
        git(&base, &["status", "--porcelain", "--untracked-files=no"]).contains("M base-only.txt")
    );
}

#[tokio::test]
async fn perform_merge_allows_untracked_files_in_base_workspace() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;

    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("feature.txt"), "from session\n")
        .expect("write worktree change");
    git(&worktree_path, &["add", "feature.txt"]);
    git(&worktree_path, &["commit", "-m", "session branch commit"]);

    std::fs::write(base.join("local-note.txt"), "local scratch\n")
        .expect("write untracked base file");

    service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::Merge,
            None,
            Some("Merge session branch".to_string()),
        )
        .await
        .expect("merge worktree with untracked base file");

    assert!(
        base.join("feature.txt").exists(),
        "committed session change should be merged"
    );
    assert!(
        base.join("local-note.txt").exists(),
        "untracked base file should remain untouched"
    );
}

#[tokio::test]
async fn perform_merge_allows_uncommitted_base_changes_when_git_can_merge() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);
    std::fs::write(base.join("base-only.txt"), "base\n").expect("write base file");
    git(&base, &["add", "base-only.txt"]);
    git(&base, &["commit", "-m", "add base-only file"]);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;

    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("feature.txt"), "from session\n")
        .expect("write worktree change");
    git(&worktree_path, &["add", "feature.txt"]);
    git(&worktree_path, &["commit", "-m", "session branch commit"]);

    std::fs::write(base.join("base-only.txt"), "local edit\n").expect("dirty base file");

    service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::Merge,
            None,
            Some("Merge session branch".to_string()),
        )
        .await
        .expect("merge worktree with unrelated dirty base file");

    assert!(
        base.join("feature.txt").exists(),
        "committed session change should be merged"
    );
    assert_eq!(
        std::fs::read_to_string(base.join("base-only.txt")).expect("read base file"),
        "local edit\n",
        "unrelated base workspace edits should be preserved"
    );
    let status = git(&base, &["status", "--porcelain", "--untracked-files=no"]);
    assert!(
        status.contains("M base-only.txt"),
        "base workspace edit should remain uncommitted"
    );
    assert_eq!(
        git(&base, &["show", "HEAD:base-only.txt"]),
        "base",
        "merge commit should not include pre-existing base workspace edits"
    );
}

#[tokio::test]
async fn perform_merge_surfaces_git_failure_for_overlapping_dirty_base_changes() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;

    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("README.md"), "from session\n")
        .expect("write worktree change");
    git(&worktree_path, &["add", "README.md"]);
    git(&worktree_path, &["commit", "-m", "session readme change"]);

    std::fs::write(base.join("README.md"), "local edit\n").expect("dirty base readme");

    let err = service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::Merge,
            None,
            Some("Merge session branch".to_string()),
        )
        .await
        .expect_err("git should reject overlapping dirty base file");

    match err {
        SessionWorktreeError::GitCommand(message) => {
            assert!(
                message.contains("README.md"),
                "git failure should include the affected file: {message}"
            );
        }
        other => panic!("expected git failure, got {other:?}"),
    }

    let latest = service
        .get_latest_for_session(session_id)
        .await
        .expect("refresh worktree")
        .expect("worktree row");
    assert_eq!(latest.status, SessionWorktreeStatus::Dirty);
    assert_eq!(
        std::fs::read_to_string(base.join("README.md")).expect("read base file"),
        "local edit\n",
        "failed merge should preserve local base workspace edits"
    );
}

#[tokio::test]
async fn perform_merge_does_not_auto_commit_uncommitted_session_changes() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().expect("temp dir");
    let base = tmp.path().join("base");
    init_git_repo(&base);
    std::fs::write(base.join(".gitignore"), ".openteams\n").expect("write gitignore");
    git(&base, &["add", ".gitignore"]);
    git(&base, &["commit", "-m", "ignore runtime files"]);

    let (worktree, _cleanup) = ensure_test_worktree(&service, session_id, base.clone()).await;

    let worktree_path = PathBuf::from(&worktree.worktree_path);
    std::fs::write(worktree_path.join("committed.txt"), "committed\n")
        .expect("write committed change");
    git(&worktree_path, &["add", "committed.txt"]);
    git(
        &worktree_path,
        &["commit", "-m", "committed session change"],
    );
    std::fs::write(worktree_path.join("uncommitted.txt"), "not merged\n")
        .expect("write uncommitted change");
    std::fs::create_dir_all(worktree_path.join(".openteams")).expect("runtime dir");
    std::fs::write(
        worktree_path.join(".openteams").join("context.jsonl"),
        "{}\n",
    )
    .expect("runtime file");

    service
        .perform_merge(
            session_id,
            SessionWorktreeMergeOperation::Merge,
            None,
            Some("Merge committed session changes".to_string()),
        )
        .await
        .expect("merge committed worktree changes");

    assert!(
        base.join("committed.txt").exists(),
        "committed branch change should be merged"
    );
    assert!(
        !base.join("uncommitted.txt").exists(),
        "uncommitted worktree change must not be auto-committed or merged"
    );
    assert!(
        !base.join(".openteams").exists(),
        "ignored runtime files must not be auto-committed or merged"
    );
    let status = git(
        &worktree_path,
        &["status", "--porcelain", "--untracked-files=all"],
    );
    assert!(
        status.contains("uncommitted.txt"),
        "uncommitted session worktree changes should remain local"
    );
}

#[tokio::test]
async fn service_merge_rejects_illegal_transition_from_non_mergeable_status() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    // `cleanup_pending` is in the active-lookup set but cannot legally
    // transition to `merging`. The reducer must reject this even though
    // the row is technically still non-terminal.
    seed_row(&pool, session_id, SessionWorktreeStatus::CleanupPending).await;

    let err = service
        .merge_session_changes(session_id, SessionWorktreeMergeOperation::Merge, None)
        .await
        .expect_err("cleanup_pending row cannot enter merging");
    assert!(matches!(
        err,
        SessionWorktreeError::IllegalTransition {
            from: SessionWorktreeStatus::CleanupPending,
            to: SessionWorktreeStatus::Merging,
            session_id: sid,
        } if sid == session_id
    ));
}

#[tokio::test]
async fn service_merge_rejects_unchanged_merged_worktree_until_it_becomes_dirty() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    // A merged worktree remains the active workspace, but it must be promoted
    // back to `dirty` before a repeat merge can start.
    seed_row(&pool, session_id, SessionWorktreeStatus::Merged).await;

    let err = service
        .merge_session_changes(session_id, SessionWorktreeMergeOperation::Merge, None)
        .await
        .expect_err("unchanged merged row cannot enter merging directly");
    assert!(matches!(
        err,
        SessionWorktreeError::IllegalTransition {
            from: SessionWorktreeStatus::Merged,
            to: SessionWorktreeStatus::Merging,
            session_id: sid,
        } if sid == session_id
    ));
}

#[tokio::test]
async fn service_cleanup_merged_is_disabled_with_active_worktree() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    // The legacy merged-cleanup route is disabled before it inspects rows, so
    // an active worktree stays untouched.
    seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let err = service
        .cleanup_merged_worktree(session_id)
        .await
        .expect_err("merged cleanup route is disabled");
    assert!(matches!(
        err,
        SessionWorktreeError::MergedCleanupRequiresDiscard(sid) if sid == session_id
    ));
    // The active row is untouched.
    let remaining = SessionWorktree::find_active_by_session(&pool, session_id)
        .await
        .unwrap()
        .expect("active row preserved");
    assert_eq!(remaining.status, SessionWorktreeStatus::Active);
}

#[tokio::test]
async fn service_cleanup_merged_is_disabled_without_merged_row() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    // The legacy merged-cleanup route is disabled even when there is no row.
    let err = service
        .cleanup_merged_worktree(session_id)
        .await
        .expect_err("merged cleanup route is disabled");
    assert!(matches!(
        err,
        SessionWorktreeError::MergedCleanupRequiresDiscard(sid) if sid == session_id
    ));
}

#[tokio::test]
async fn service_discard_can_remove_merged_worktree() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let merged = seed_row(&pool, session_id, SessionWorktreeStatus::Merged).await;
    let session_agent_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO chat_session_agents (
            id, session_id, agent_id, state, workspace_path
        )
        VALUES (?1, ?2, ?3, 'idle', ?4)
        "#,
    )
    .bind(session_agent_id)
    .bind(session_id)
    .bind(Uuid::new_v4())
    .bind(merged.worktree_path.clone())
    .execute(&pool)
    .await
    .expect("insert session agent");

    // WorktreeManager::cleanup_worktree on non-existent paths falls back to
    // simple_worktree_cleanup which is a no-op when the directory does not
    // exist — so the success path is exercisable in-memory.
    let result = service
        .discard_worktree(session_id)
        .await
        .expect("merged row should clean up through discard");

    assert_eq!(result.status, SessionWorktreeStatus::Archived);
    assert_eq!(result.id, merged.id);
    assert!(result.archived_at.is_some());

    // The merged row is gone from the active lookup but remains in audit
    // history, now in `archived` terminal state.
    let audit = SessionWorktree::find_latest_by_session_and_status(
        &pool,
        session_id,
        SessionWorktreeStatus::Archived,
    )
    .await
    .unwrap()
    .expect("archived audit row present");
    assert_eq!(audit.id, merged.id);
    assert!(
        SessionWorktree::find_active_by_session(&pool, session_id)
            .await
            .unwrap()
            .is_none()
    );

    let workspace_path: String =
        sqlx::query_scalar("SELECT workspace_path FROM chat_session_agents WHERE id = ?1")
            .bind(session_agent_id)
            .fetch_one(&pool)
            .await
            .expect("read restored workspace path");
    assert_eq!(workspace_path, merged.base_workspace_path);
}

#[tokio::test]
async fn service_retry_cleanup_refuses_session_without_failed_row() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    // No active row and no cleanup_failed row: the guard passes but the
    // targeted lookup returns None, surfacing NoCleanupFailedWorktree.
    let err = service
        .retry_cleanup(session_id)
        .await
        .expect_err("no cleanup_failed row -> refuse retry");
    assert!(matches!(
        err,
        SessionWorktreeError::NoCleanupFailedWorktree(sid) if sid == session_id
    ));
}

#[tokio::test]
async fn service_retry_cleanup_refuses_session_with_active_worktree() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    // Both a cleanup_failed historical row AND an active row exist for the
    // same session. Because worktree_path_for_session is derived only from
    // the session id, both rows share the same physical path. Retrying
    // cleanup of the failed row would delete the active worktree — the
    // path-collision safety guard must refuse.
    let _failed = seed_row(&pool, session_id, SessionWorktreeStatus::CleanupFailed).await;
    let active = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let err = service
        .retry_cleanup(session_id)
        .await
        .expect_err("active row -> refuse retry to avoid path collision");
    assert!(matches!(
        err,
        SessionWorktreeError::SessionHasActiveWorktree(sid) if sid == session_id
    ));
    // Both rows are untouched: the cleanup_failed row is still failed, the
    // active row is still active.
    let remaining_active = SessionWorktree::find_active_by_session(&pool, session_id)
        .await
        .unwrap()
        .expect("active row preserved");
    assert_eq!(remaining_active.id, active.id);
    assert_eq!(remaining_active.status, SessionWorktreeStatus::Active);
    let remaining_failed = SessionWorktree::find_latest_by_session_and_status(
        &pool,
        session_id,
        SessionWorktreeStatus::CleanupFailed,
    )
    .await
    .unwrap()
    .expect("cleanup_failed row preserved");
    assert_eq!(
        remaining_failed.status,
        SessionWorktreeStatus::CleanupFailed
    );
}

#[tokio::test]
async fn service_retry_cleanup_recovers_cleanup_failed_row_to_archived() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let failed = seed_row(&pool, session_id, SessionWorktreeStatus::CleanupFailed).await;
    // Seed a cleanup_error so we can verify it is cleared on success.
    SessionWorktree::set_cleanup_error(&pool, failed.id, "previous attempt failed")
        .await
        .unwrap();

    let result = service
        .retry_cleanup(session_id)
        .await
        .expect("cleanup_failed row should retry successfully");

    assert_eq!(result.status, SessionWorktreeStatus::Archived);
    assert_eq!(result.id, failed.id);
    assert!(result.archived_at.is_some());
    // transition_status_clearing_transient wipes cleanup_error on the way
    // through cleanup_pending; archived row should not retain stale error.
    let archived = SessionWorktree::find_by_id(&pool, failed.id)
        .await
        .unwrap()
        .unwrap();
    assert!(archived.cleanup_error.is_none());
}

#[tokio::test]
async fn service_get_effective_workspace_returns_none_for_legacy_session() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();

    // No row at all: legacy session must take the main-workspace fallback.
    // This is the critical "do not change resolver path for non-isolated
    // sessions" guarantee.
    assert_eq!(
        service.get_effective_workspace(session_id).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn service_get_effective_workspace_returns_worktree_path() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let row = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    let path = service
        .get_effective_workspace(session_id)
        .await
        .unwrap()
        .expect("active worktree exposes a path");
    assert_eq!(path, PathBuf::from(&row.worktree_path));
}

#[tokio::test]
async fn service_get_effective_workspace_returns_none_during_cleanup() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    seed_row(&pool, session_id, SessionWorktreeStatus::CleanupPending).await;

    assert_eq!(
        service.get_effective_workspace(session_id).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn service_ensure_returns_existing_without_touching_filesystem() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let existing = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;

    // ensure_for_session must NOT try to resolve a git repo or call the
    // WorktreeManager when an active row already exists. We pass a bogus
    // base path to make sure that code path is not reached.
    let outcome = service
        .ensure_for_session(EnsureWorktreeInput::new(
            session_id,
            PathBuf::from("/nonexistent/path"),
        ))
        .await
        .expect("idempotent ensure");

    match outcome {
        EnsureOutcome::Existing(row) => {
            assert_eq!(row.id, existing.id);
        }
        EnsureOutcome::Created(_) => panic!("must not create a duplicate worktree"),
    }
}

#[tokio::test]
async fn service_ensure_syncs_session_agent_workspace_paths_to_worktree() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let existing = seed_row(&pool, session_id, SessionWorktreeStatus::Active).await;
    let session_agent_id = Uuid::new_v4();

    sqlx::query(
        r#"
        INSERT INTO chat_session_agents (
            id, session_id, agent_id, state, workspace_path
        )
        VALUES (?1, ?2, ?3, 'idle', ?4)
        "#,
    )
    .bind(session_agent_id)
    .bind(session_id)
    .bind(Uuid::new_v4())
    .bind(existing.base_workspace_path.clone())
    .execute(&pool)
    .await
    .expect("insert session agent");

    service
        .ensure_for_session(EnsureWorktreeInput::new(
            session_id,
            PathBuf::from("/nonexistent/path"),
        ))
        .await
        .expect("idempotent ensure syncs paths");

    let workspace_path: String =
        sqlx::query_scalar("SELECT workspace_path FROM chat_session_agents WHERE id = ?1")
            .bind(session_agent_id)
            .fetch_one(&pool)
            .await
            .expect("read synced workspace path");
    assert_eq!(workspace_path, existing.worktree_path);
}

#[tokio::test]
async fn service_ensure_rejects_non_git_base_and_leaves_no_row() {
    let pool = setup_pool().await;
    let service = SessionWorktreeService::new(pool.clone());
    let session_id = Uuid::new_v4();
    let tmp = tempfile::TempDir::new().unwrap();
    let base = tmp.path().canonicalize().unwrap();

    let err = service
        .ensure_for_session(EnsureWorktreeInput::new(session_id, base.clone()))
        .await
        .expect_err("non-git base must be rejected");
    assert!(matches!(err, SessionWorktreeError::NotAGitRepo(p) if p == base));
    // No row should have been written.
    assert!(
        SessionWorktree::find_active_by_session(&pool, session_id)
            .await
            .unwrap()
            .is_none()
    );
}

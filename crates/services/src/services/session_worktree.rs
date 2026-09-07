use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use db::models::chat_session_worktree::{
    CreateSessionWorktree, SessionWorktree, SessionWorktreeError as ModelCasError,
    SessionWorktreeMergeOperation, SessionWorktreeMode, SessionWorktreeStatus,
};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Sqlite};
use thiserror::Error;
use tokio::{
    process::Command,
    time::{Duration, timeout},
};
use tracing::{debug, info, warn};
use ts_rs::TS;
use uuid::Uuid;

use super::{
    inbox::InboxService,
    worktree_manager::{WorktreeCleanup, WorktreeError, WorktreeManager},
};

/// Length of the short session-id suffix used in branch names and worktree
/// paths. Kept short to avoid Windows path-length issues and to produce
/// human-scannable branch names. 8 hex chars = 4 bytes of entropy from the
/// UUID; collision risk inside a single installation is negligible.
const SHORT_ID_LEN: usize = 8;

/// Branch prefix for session worktrees. Stable so users can recognise these
/// branches in their git UI and so reconciliation can identify orphans.
const SESSION_BRANCH_PREFIX: &str = "openteams/session/";

const GIT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Error)]
pub enum SessionWorktreeError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(transparent)]
    WorktreeManager(#[from] WorktreeError),
    #[error(transparent)]
    ModelCas(#[from] ModelCasError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("session {0} has no active worktree")]
    NoActiveWorktree(Uuid),
    #[error("session {0} has no merged worktree to clean up")]
    NoMergedWorktree(Uuid),
    #[error("merged session worktrees can only be cleaned up through discard")]
    MergedCleanupRequiresDiscard(Uuid),
    #[error("session {0} has no cleanup_failed worktree to retry")]
    NoCleanupFailedWorktree(Uuid),
    #[error(
        "session {0} still has an active worktree; refuse auto cleanup to avoid path collision"
    )]
    SessionHasActiveWorktree(Uuid),
    #[error("illegal session worktree transition: session={session_id} from={from:?} to={to:?}")]
    IllegalTransition {
        session_id: Uuid,
        from: SessionWorktreeStatus,
        to: SessionWorktreeStatus,
    },
    #[error("base workspace is not a git repository: {0}")]
    NotAGitRepo(PathBuf),
    #[error("base workspace is not on the expected branch '{expected}'; currently on '{actual}'")]
    BaseWorkspaceWrongBranch { expected: String, actual: String },
    #[error("a git merge or rebase is already in progress in the base workspace")]
    MergeOperationInProgress,
    #[error("session {0} has no merge in progress to continue or abort")]
    NoMergeInProgress(Uuid),
    #[error("unresolved conflicts remain: {0:?}")]
    UnresolvedConflicts(Vec<String>),
    #[error("git command failed: {0}")]
    GitCommand(String),
    #[error("conflict file path is invalid or escapes the workspace: {0}")]
    InvalidConflictPath(String),
}

/// Outcome of `read_conflict_file`: the three Git index stages plus the
/// working-tree content. The App's merge editor renders `base` (common
/// ancestor), `current` (ours / base branch), and `session` (theirs /
/// session branch) side-by-side. `working_tree` contains the current file
/// on disk (which may still have conflict markers).
///
/// Fields are `Option` because Git may not have all three stages for every
/// conflict (e.g. add/add has no base, delete/modify has no theirs).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConflictFileContent {
    pub path: String,
    #[ts(optional, type = "string | null")]
    pub base: Option<String>,
    #[ts(optional, type = "string | null")]
    pub current: Option<String>,
    #[ts(optional, type = "string | null")]
    pub session: Option<String>,
    pub working_tree: String,
    pub is_binary: bool,
    pub is_too_large: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ConflictResolutionSide {
    Current,
    Session,
}

/// Summary of a single conflicted path, for the conflict-file list view.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct ConflictFileInfo {
    pub path: String,
    /// Human-readable conflict status, e.g. `both_modified`, `deleted_by_us`,
    /// `deleted_by_them`, `added_by_us`, `added_by_them`. Derived from Git's
    /// unmerged porcelain status so the UI can pick text or file-level flow.
    pub status: String,
}

const MAX_CONFLICT_TEXT_BYTES: usize = 256 * 1024;

struct ConflictContentPart {
    text: Option<String>,
    is_binary: bool,
    is_too_large: bool,
}

fn conflict_content_part(bytes: Option<&[u8]>) -> ConflictContentPart {
    let Some(bytes) = bytes else {
        return ConflictContentPart {
            text: None,
            is_binary: false,
            is_too_large: false,
        };
    };

    let is_too_large = bytes.len() > MAX_CONFLICT_TEXT_BYTES;
    let is_binary = conflict_bytes_are_binary(bytes);
    let text = if is_too_large || is_binary {
        Some(String::new())
    } else {
        Some(String::from_utf8_lossy(bytes).to_string())
    };

    ConflictContentPart {
        text,
        is_binary,
        is_too_large,
    }
}

fn conflict_bytes_are_binary(bytes: &[u8]) -> bool {
    bytes.contains(&0) || std::str::from_utf8(bytes).is_err()
}

fn parse_unmerged_porcelain(output: &str) -> Vec<ConflictFileInfo> {
    output
        .lines()
        .filter_map(|line| {
            let code = line.get(0..2)?;
            let raw_path = line.get(3..)?.trim();
            let status = conflict_status_from_porcelain(code, raw_path)?;
            let path = raw_path
                .rsplit_once(" -> ")
                .map(|(_, new_path)| new_path)
                .unwrap_or(raw_path)
                .trim_matches('"')
                .to_string();
            if path.is_empty() {
                return None;
            }
            Some(ConflictFileInfo {
                path,
                status: status.to_string(),
            })
        })
        .collect()
}

fn conflict_status_from_porcelain(code: &str, path: &str) -> Option<&'static str> {
    match code {
        "UU" => Some("both_modified"),
        "AA" => Some("both_added"),
        "DD" => Some("both_deleted"),
        "DU" => Some("deleted_by_us"),
        "UD" => Some("deleted_by_them"),
        "AU" => Some("added_by_us"),
        "UA" => Some("added_by_them"),
        _ if code.contains('R') || path.contains(" -> ") => Some("renamed"),
        _ => None,
    }
}

/// Result of `perform_merge`: tells the caller whether the merge completed
/// cleanly or has conflicts that need App-internal resolution.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
pub struct MergeResult {
    pub worktree: SessionWorktree,
    pub has_conflicts: bool,
    pub conflict_files: Vec<String>,
}

/// Internal outcome of the merge git operations.
enum MergeOutcome {
    Success,
    Conflicts(Vec<String>),
}

/// Input for lazy worktree creation.
///
/// The caller (chat runner / route handler) is responsible for resolving the
/// user-facing base workspace path from session / project / agent state. The
/// service intentionally does not read `chat_sessions` directly so that the
/// "isolated enabled?" decision stays in the caller — sessions that have not
/// opted into isolation never invoke this code path, which keeps the
/// resolver behaviour unchanged for legacy sessions.
#[derive(Debug, Clone)]
pub struct EnsureWorktreeInput {
    pub session_id: Uuid,
    pub project_id: Option<Uuid>,
    /// User-facing main workspace path the worktree is isolating from.
    /// Typically `chat_sessions.default_workspace_path` or a project default.
    pub base_workspace_path: PathBuf,
}

impl EnsureWorktreeInput {
    pub fn new(session_id: Uuid, base_workspace_path: PathBuf) -> Self {
        Self {
            session_id,
            project_id: None,
            base_workspace_path,
        }
    }

    pub fn with_project(mut self, project_id: Option<Uuid>) -> Self {
        self.project_id = project_id;
        self
    }
}

/// Outcome of `ensure_for_session`. `Existing` is returned when the session
/// already has an active worktree row so the caller can distinguish "we just
/// created it (run setup steps)" from "idempotent re-entry (skip setup)".
#[derive(Debug, Clone)]
pub enum EnsureOutcome {
    Created(SessionWorktree),
    Existing(SessionWorktree),
}

/// Authoritative state machine for `SessionWorktreeStatus`.
///
/// Returns `Ok(())` only for legal `from -> to` pairs. Any pair not listed
/// here is rejected; the rejected target is returned in `Err` so tests can
/// assert which transition was refused.
///
/// Legal transitions:
/// - `creating` -> `active` | `cleanup_failed` | `archived`
/// - `active`   -> `dirty` | `merging` | `cleanup_pending` | `archived` | `cleanup_failed`
/// - `dirty`    -> `active` | `merging` | `cleanup_pending` | `archived` | `cleanup_failed`
/// - `merging`  -> `merged` | `needs_conflict_resolution` | `dirty` | `active`
///   | `cleanup_pending` | `cleanup_failed`
/// - `needs_conflict_resolution` -> `merged` | `dirty` | `merging`
///   | `cleanup_pending` | `cleanup_failed`
/// - `merged`   -> `dirty` | `cleanup_pending` | `archived`
/// - `cleanup_pending` -> `archived` | `cleanup_failed`
/// - `cleanup_failed`  -> `cleanup_pending` | `archived`
/// - `archived` is terminal.
///
/// Self-transitions are legal (treated as idempotent retries).
pub fn validate_transition(
    from: SessionWorktreeStatus,
    to: SessionWorktreeStatus,
) -> Result<(), SessionWorktreeStatus> {
    use SessionWorktreeStatus::*;
    if from == to {
        return Ok(());
    }
    let legal = match from {
        Creating => matches!(to, Active | CleanupFailed | Archived),
        Active => matches!(
            to,
            Dirty | Merging | CleanupPending | Archived | CleanupFailed
        ),
        Dirty => matches!(
            to,
            Active | Merging | CleanupPending | Archived | CleanupFailed
        ),
        Merging => matches!(
            to,
            Merged | NeedsConflictResolution | Dirty | Active | CleanupPending | CleanupFailed
        ),
        NeedsConflictResolution => {
            matches!(
                to,
                Merged | Dirty | Merging | CleanupPending | CleanupFailed
            )
        }
        Merged => matches!(to, Dirty | CleanupPending | Archived),
        CleanupPending => matches!(to, Archived | CleanupFailed),
        CleanupFailed => matches!(to, CleanupPending | Archived),
        Archived => false,
    };
    if legal { Ok(()) } else { Err(to) }
}

/// Returns true when the worktree row may be force-removed by an automated
/// (non-user-initiated) cleanup pass. Mirrors the safety rule in the design
/// doc: an unmerged worktree with active runtime state must never be cleaned
/// up automatically — only an explicit user `discard_worktree` call may
/// proceed.
pub fn is_safe_for_auto_cleanup(status: SessionWorktreeStatus) -> bool {
    use SessionWorktreeStatus::*;
    matches!(status, CleanupPending | CleanupFailed | Archived)
}

/// Short, stable suffix derived from a session id. Used both in branch names
/// and worktree path segments so a user can map between them by eye.
///
/// Returns the first `SHORT_ID_LEN` hex characters of the simple uuid string
/// (no hyphens), lowercased.
pub fn short_session_id(session_id: Uuid) -> String {
    session_id
        .simple()
        .to_string()
        .to_ascii_lowercase()
        .chars()
        .take(SHORT_ID_LEN)
        .collect()
}

/// Stable branch name for a session worktree: `openteams/session/<short-id>`.
///
/// Stable across runs of the same session: re-calling `ensure_for_session`
/// for an existing active worktree must produce the same branch so that
/// `WorktreeManager::ensure_worktree_exists` is idempotent.
pub fn branch_name_for_session(session_id: Uuid) -> String {
    format!("{SESSION_BRANCH_PREFIX}{}", short_session_id(session_id))
}

/// Worktree path under the app-managed session worktree base dir.
///
/// Layout: `<session_worktree_base>/<short-id>`. By default the session base is
/// `<worktree_base>/sessions`; users may override it from settings.
pub fn worktree_path_for_session(session_id: Uuid) -> PathBuf {
    WorktreeManager::get_session_worktree_base_dir().join(short_session_id(session_id))
}

/// Walk up from `start` looking for a `.git` entry. Returns the first
/// directory that contains one (the main repo workdir), or `None` if we
/// reach the filesystem root without finding one.
///
/// Accepts both bare `.git` directories (normal clones) and `.git` files
/// (which point at a gitdir, e.g. in worktrees or submodules).
pub fn detect_git_repo_path(start: &Path) -> Option<PathBuf> {
    let mut current = match start.canonicalize() {
        Ok(p) => p,
        Err(_) => start.to_path_buf(),
    };
    loop {
        if current.join(".git").exists() {
            return Some(current);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => return None,
        }
    }
}

/// Resolve the checked-out branch for the base workspace. This is the branch
/// a session worktree should fork from because isolated sessions are scoped to
/// the user's current project workspace, not the remote's default branch.
pub async fn detect_current_branch(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["branch", "--show-current"])
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !line.is_empty() {
            return Some(line);
        }
    }
    None
}

/// Resolve the repository default branch name as a fallback for detached HEAD
/// or unusual Git states. Strategy: ask `git symbolic-ref
/// refs/remotes/origin/HEAD` first, fall back to the local HEAD. Returns
/// `None` if git is unavailable; callers fall back to a literal default
/// (typically `main`).
pub async fn detect_default_branch(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if let Some(stripped) = line.strip_prefix("origin/") {
            return Some(stripped.to_string());
        }
        if !line.is_empty() {
            return Some(line);
        }
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["symbolic-ref", "--short", "HEAD"])
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !line.is_empty() {
            return Some(line);
        }
    }
    None
}

/// Resolve HEAD commit sha for `branch` in `repo_path`. Used to initialize the
/// worktree integration cursor. Returns `None` if git is unavailable.
pub async fn detect_branch_head(repo_path: &Path, branch: &str) -> Option<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_path)
        .args(["rev-parse", branch])
        .output()
        .await
        .ok()?;
    if output.status.success() {
        let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !line.is_empty() {
            return Some(line);
        }
    }
    None
}

/// Validate that a conflict-file path is relative and does not escape the
/// workspace via `..` components. This prevents path-traversal attacks via
/// the `merge-conflicts/{path}` route.
fn validate_conflict_path(path: &str) -> Result<(), SessionWorktreeError> {
    let p = Path::new(path);
    let bytes = path.as_bytes();
    let has_windows_drive_prefix = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    if p.is_absolute()
        || has_windows_drive_prefix
        || path.starts_with("\\\\")
        || p.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        })
    {
        return Err(SessionWorktreeError::InvalidConflictPath(path.to_string()));
    }
    Ok(())
}

/// Service: the only legal writer of `chat_session_worktrees.status`.
///
/// Holds a DB pool handle; cheap to clone. Routes should construct one per
/// request; the chat runner should hold one for the lifetime of the session.
#[derive(Clone)]
pub struct SessionWorktreeService {
    pool: Pool<Sqlite>,
}

impl SessionWorktreeService {
    pub fn new(pool: Pool<Sqlite>) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &Pool<Sqlite> {
        &self.pool
    }

    /// Lazy-create (or return the existing) worktree for a session.
    ///
    /// Idempotent: if the session already has a non-terminal worktree row,
    /// returns it without touching the filesystem. The first call performs
    /// the actual `git worktree add` via `WorktreeManager`.
    ///
    /// Caller responsibility: only invoke this when the session has opted
    /// into isolation. Sessions without isolation enabled must continue to
    /// use `chat_session_agents.workspace_path` / `default_workspace_path`
    /// directly — this method never silently enables isolation.
    pub async fn ensure_for_session(
        &self,
        input: EnsureWorktreeInput,
    ) -> Result<EnsureOutcome, SessionWorktreeError> {
        if let Some(existing) =
            SessionWorktree::find_active_by_session(&self.pool, input.session_id).await?
        {
            if existing.status.is_active_for_workspace() {
                self.sync_session_agent_workspace_paths(input.session_id, &existing.worktree_path)
                    .await?;
            }
            return Ok(EnsureOutcome::Existing(existing));
        }

        let repo_path = detect_git_repo_path(&input.base_workspace_path).ok_or(
            SessionWorktreeError::NotAGitRepo(input.base_workspace_path.clone()),
        )?;

        let base_branch = match detect_current_branch(&input.base_workspace_path).await {
            Some(branch) => branch,
            None => detect_default_branch(&repo_path)
                .await
                .unwrap_or_else(|| "main".to_string()),
        };

        let branch_name = branch_name_for_session(input.session_id);
        let worktree_path = worktree_path_for_session(input.session_id);

        let row = SessionWorktree::create(
            &self.pool,
            &CreateSessionWorktree {
                session_id: input.session_id,
                project_id: input.project_id,
                base_workspace_path: input.base_workspace_path.to_string_lossy().to_string(),
                repo_path: repo_path.to_string_lossy().to_string(),
                base_branch: base_branch.clone(),
                base_commit: None,
                branch_name: branch_name.clone(),
                worktree_path: worktree_path.to_string_lossy().to_string(),
                mode: SessionWorktreeMode::Session,
            },
            Uuid::new_v4(),
        )
        .await?;

        // Stamp base_commit best-effort before the worktree is created so the
        // audit row is correct even if creation later fails.
        if let Some(head) = detect_branch_head(&repo_path, &base_branch).await
            && let Err(err) =
                SessionWorktree::record_base_commit(&self.pool, row.id, Some(&head)).await
        {
            warn!(
                worktree_id = %row.id,
                error = %err,
                "Failed to record base_commit; continuing"
            );
        }

        match WorktreeManager::create_worktree(
            &repo_path,
            &branch_name,
            &worktree_path,
            &base_branch,
            true,
        )
        .await
        {
            Ok(()) => {
                let active = Self::apply_transition(
                    &self.pool,
                    row.id,
                    input.session_id,
                    SessionWorktreeStatus::Creating,
                    SessionWorktreeStatus::Active,
                )
                .await?;
                info!(
                    worktree_id = %active.id,
                    session_id = %input.session_id,
                    branch = %branch_name,
                    "Created session worktree"
                );
                self.sync_session_agent_workspace_paths(input.session_id, &active.worktree_path)
                    .await?;
                Ok(EnsureOutcome::Created(active))
            }
            Err(err) => {
                let message = err.to_string();
                warn!(
                    worktree_id = %row.id,
                    session_id = %input.session_id,
                    error = %message,
                    "Session worktree creation failed"
                );
                // Move the row to cleanup_failed (excluded from
                // find_active_by_session) so the user can retry or the
                // janitor can reconcile. We do not delete the row: preserving
                // it gives the next step (route/runner) enough context to
                // explain the failure.
                if let Err(cas_err) = SessionWorktree::transition_status(
                    &self.pool,
                    row.id,
                    SessionWorktreeStatus::Creating,
                    SessionWorktreeStatus::CleanupFailed,
                )
                .await
                {
                    debug!(
                        worktree_id = %row.id,
                        error = ?cas_err,
                        "Could not transition failed-creation row to cleanup_failed"
                    );
                }
                let _ = SessionWorktree::set_cleanup_error(&self.pool, row.id, &message).await;
                Err(SessionWorktreeError::WorktreeManager(err))
            }
        }
    }

    /// Resolve the workspace path an agent run / source-control read should
    /// use for this session. Returns `Some(worktree_path)` when the session
    /// has a non-terminal worktree, `None` otherwise.
    ///
    /// Callers (chat runner resolver, source-control endpoint) MUST fall
    /// back to the existing main-workspace logic when this returns `None`.
    /// Sessions that have not opted into isolation never get a row, so they
    /// naturally take the legacy path and no new Git operations are
    /// introduced.
    pub async fn get_effective_workspace(
        &self,
        session_id: Uuid,
    ) -> Result<Option<PathBuf>, SessionWorktreeError> {
        let row = self.get_latest_for_session(session_id).await?;
        Ok(row
            .filter(|r| r.status.is_active_for_workspace())
            .map(|r| PathBuf::from(r.worktree_path)))
    }

    /// Read the active row for a session. Used by routes that need to render
    /// badge state (e.g. `cleanup_failed` shows a Retry button).
    pub async fn get_for_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<SessionWorktree>, SessionWorktreeError> {
        Ok(SessionWorktree::find_active_by_session(&self.pool, session_id).await?)
    }

    /// Read the most recent worktree row for a session regardless of status.
    /// Used by the status route so the UI can render terminal states
    /// (`merged`, `cleanup_failed`, `archived`) and expose the retry-cleanup
    /// entry point. Returns `None` when the session has never had a worktree.
    pub async fn get_latest_for_session(
        &self,
        session_id: Uuid,
    ) -> Result<Option<SessionWorktree>, SessionWorktreeError> {
        let Some(row) = SessionWorktree::find_latest_for_session(&self.pool, session_id).await?
        else {
            return Ok(None);
        };
        Ok(Some(self.refresh_merge_availability(row).await?))
    }

    async fn refresh_merge_availability(
        &self,
        mut row: SessionWorktree,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let has_unmerged_commits = if row.status.is_active_for_workspace() {
            self.branch_has_commits_not_in_base(&row).await?
        } else {
            false
        };
        row.has_unmerged_commits = has_unmerged_commits;

        if row.status != SessionWorktreeStatus::Merged || !has_unmerged_commits {
            if row.status.is_active_for_workspace() {
                self.sync_session_agent_workspace_paths(row.session_id, &row.worktree_path)
                    .await?;
            }
            return Ok(row);
        }

        let mut dirty = Self::apply_transition(
            &self.pool,
            row.id,
            row.session_id,
            SessionWorktreeStatus::Merged,
            SessionWorktreeStatus::Dirty,
        )
        .await?;
        dirty.has_unmerged_commits = true;
        self.sync_session_agent_workspace_paths(dirty.session_id, &dirty.worktree_path)
            .await?;
        Ok(dirty)
    }

    async fn sync_session_agent_workspace_paths(
        &self,
        session_id: Uuid,
        worktree_path: &str,
    ) -> Result<(), SessionWorktreeError> {
        sqlx::query(
            r#"
            UPDATE chat_session_agents
            SET workspace_path = ?2,
                updated_at = datetime('now', 'subsec')
            WHERE session_id = ?1
              AND COALESCE(workspace_path, '') <> ?2
            "#,
        )
        .bind(session_id)
        .bind(worktree_path)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Begin a merge of the session worktree's changes back into a target
    /// branch in the main repo.
    ///
    /// **Phase 1 skeleton**: validates preconditions, transitions the row to
    /// `merging`, and records the merge intent (operation + target branch +
    /// `operation_started_at`). The actual git operations (`git merge`,
    /// `cherry-pick`, `rebase`) and conflict detection live in a
    /// follow-up task; this reducer layer is stable so the
    /// `merge/continue` / `merge/abort` / `merge-conflicts/resolve` routes
    /// can be wired against it next.
    ///
    /// Returns the row now in `merging` status. The follow-up merge executor
    /// will call `mark_merged` or `mark_needs_conflict_resolution` after
    /// running git.
    pub async fn merge_session_changes(
        &self,
        session_id: Uuid,
        operation: SessionWorktreeMergeOperation,
        target_branch: Option<String>,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;

        validate_transition(row.status, SessionWorktreeStatus::Merging).map_err(|to| {
            SessionWorktreeError::IllegalTransition {
                session_id,
                from: row.status,
                to,
            }
        })?;

        // CAS the status first; only persist intent metadata on success so a
        // rejected CAS does not leave stale `merge_operation`/`target_branch`
        // on a row that did not enter `merging`.
        let merging_row = Self::apply_transition(
            &self.pool,
            row.id,
            session_id,
            row.status,
            SessionWorktreeStatus::Merging,
        )
        .await?;

        SessionWorktree::set_merge_metadata(
            &self.pool,
            merging_row.id,
            operation,
            target_branch.as_deref(),
        )
        .await
        .map_err(Into::into)
    }

    /// Mark a merge as completed. Called by the merge executor after a
    /// successful `git merge` / squash / cherry-pick / rebase.
    pub async fn mark_merged(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;
        let updated = Self::apply_transition(
            &self.pool,
            row.id,
            session_id,
            row.status,
            SessionWorktreeStatus::Merged,
        )
        .await?;
        let updated = SessionWorktree::record_merged_at(&self.pool, updated.id).await?;
        // Keep agents in the physical worktree after a successful merge. The
        // user can inspect or continue work there until an explicit discard
        // removes it and restores the base workspace path.
        self.sync_session_agent_workspace_paths(session_id, &updated.worktree_path)
            .await?;
        Ok(updated)
    }

    /// Mark a merge as having conflicts. Called by the merge executor when
    /// `git merge` reports conflicts. `conflict_files` is the list of paths
    /// (relative to the merge target) that need user resolution.
    pub async fn mark_needs_conflict_resolution(
        &self,
        session_id: Uuid,
        conflict_files: &[String],
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;
        let updated = Self::apply_transition(
            &self.pool,
            row.id,
            session_id,
            row.status,
            SessionWorktreeStatus::NeedsConflictResolution,
        )
        .await?;
        let updated =
            SessionWorktree::set_conflict_files(&self.pool, updated.id, conflict_files).await?;
        InboxService::new()
            .notify_worktree_conflict(&self.pool, &updated, conflict_files)
            .await;
        Ok(updated)
    }

    /// Abort an in-progress merge and return the worktree to its pre-merge
    /// status. Caller chooses whether to land in `dirty` (changes preserved,
    /// default) or `active` (treat as clean).
    pub async fn abort_merge(
        &self,
        session_id: Uuid,
        prefer_clean: bool,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;
        let to = if prefer_clean {
            SessionWorktreeStatus::Active
        } else {
            SessionWorktreeStatus::Dirty
        };
        validate_transition(row.status, to).map_err(|to| {
            SessionWorktreeError::IllegalTransition {
                session_id,
                from: row.status,
                to,
            }
        })?;
        SessionWorktree::transition_status_clearing_transient(&self.pool, row.id, row.status, to)
            .await
            .map_err(Into::into)
    }

    // -----------------------------------------------------------------
    // Merge executor + conflict resolution (route-facing)
    // -----------------------------------------------------------------

    /// Perform a full merge of the session worktree's changes back into the
    /// base workspace. This is the route-facing entry point that combines
    /// state transitions with actual Git operations.
    ///
    /// Flow:
    /// 1. Transition to `merging` (via `merge_session_changes`).
    /// 2. Verify the base workspace is on the base branch and has no
    ///    in-progress merge/rebase.
    /// 3. Apply the requested integration strategy in the base workspace.
    ///    The route-facing default combines the session branch into one
    ///    single-parent squash commit, so all conflicts surface together.
    /// 4. If conflicts are detected: transition to `needs_conflict_resolution`
    ///    and persist the conflict file list. The worktree is NOT deleted.
    /// 5. If no conflicts: `git commit`, transition to `merged`.
    /// 6. On unexpected error: abort the Git state and transition back to
    ///    `dirty`.
    pub async fn perform_merge(
        &self,
        session_id: Uuid,
        operation: SessionWorktreeMergeOperation,
        target_branch: Option<String>,
        commit_message: Option<String>,
    ) -> Result<MergeResult, SessionWorktreeError> {
        // 1. Load the active worktree row and run preconditions BEFORE
        //    transitioning to `merging`. We intentionally do not require a
        //    clean base workspace; Git decides whether the local changes can
        //    coexist with this merge and returns the actionable failure.
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;
        self.validate_merge_preconditions(&row).await?;

        // 2. Preconditions passed — now transition to `merging`.
        let merging_row = self
            .merge_session_changes(session_id, operation, target_branch)
            .await?;

        // 3. Execute the merge. At this point preconditions have
        //    already been validated, so the only possible outcomes are
        //    success, conflicts, or an unexpected git error.
        match self
            .execute_merge(&merging_row, commit_message.as_deref())
            .await
        {
            Ok(MergeOutcome::Success) => {
                let merged = match merging_row.merge_operation {
                    Some(
                        SessionWorktreeMergeOperation::CherryPick
                        | SessionWorktreeMergeOperation::SquashMerge,
                    ) => {
                        self.mark_cursor_integration_merged(session_id, &merging_row)
                            .await?
                    }
                    _ => self.mark_merged(session_id).await?,
                };
                Ok(MergeResult {
                    worktree: merged,
                    has_conflicts: false,
                    conflict_files: Vec::new(),
                })
            }
            Ok(MergeOutcome::Conflicts(files)) => {
                let conflict_row = self
                    .mark_needs_conflict_resolution(session_id, &files)
                    .await?;
                Ok(MergeResult {
                    worktree: conflict_row,
                    has_conflicts: true,
                    conflict_files: files,
                })
            }
            Err(err) => {
                let _ = self.abort_merge(session_id, false).await;
                Err(err)
            }
        }
    }

    /// Validate merge preconditions that Git cannot resolve during the merge
    /// itself. Base workspace changes are allowed through so unrelated local
    /// work does not block merging a session branch.
    async fn validate_merge_preconditions(
        &self,
        row: &SessionWorktree,
    ) -> Result<(), SessionWorktreeError> {
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        let base_branch = &row.base_branch;

        if detect_git_repo_path(&base_workspace).is_none() {
            return Err(SessionWorktreeError::NotAGitRepo(base_workspace));
        }

        let current_branch = self.current_branch(&base_workspace).await?;
        if current_branch.as_deref() != Some(base_branch.as_str()) {
            return Err(SessionWorktreeError::BaseWorkspaceWrongBranch {
                expected: base_branch.clone(),
                actual: current_branch.unwrap_or_default(),
            });
        }

        if self.has_operation_in_progress(&base_workspace).await? {
            return Err(SessionWorktreeError::MergeOperationInProgress);
        }

        Ok(())
    }

    /// Execute the Git merge in the base workspace. Reads the
    /// worktree row for `base_workspace_path`, `base_branch`, and
    /// `branch_name`. Preconditions are assumed to have already been
    /// validated by `validate_merge_preconditions`.
    async fn execute_merge(
        &self,
        row: &SessionWorktree,
        commit_message: Option<&str>,
    ) -> Result<MergeOutcome, SessionWorktreeError> {
        match row.merge_operation {
            Some(SessionWorktreeMergeOperation::CherryPick) => {
                return self.execute_cherry_pick(row).await;
            }
            Some(SessionWorktreeMergeOperation::SquashMerge) => {
                return self.execute_squash_merge(row, commit_message).await;
            }
            _ => {}
        }

        let base_workspace = PathBuf::from(&row.base_workspace_path);
        let session_branch = &row.branch_name;
        let pre_merge_dirty_paths = self.dirty_tracked_paths(&base_workspace).await?;

        let merge_result = self
            .run_git(
                &base_workspace,
                &["merge", "--no-ff", "--no-commit", session_branch],
            )
            .await;

        if let Err(ref _e) = merge_result {
            let conflicts = self.list_unmerged_paths(&base_workspace).await?;
            if !conflicts.is_empty() {
                return Ok(MergeOutcome::Conflicts(conflicts));
            }
            return Err(merge_result.unwrap_err());
        }

        let conflicts = self.list_unmerged_paths(&base_workspace).await?;
        if !conflicts.is_empty() {
            return Ok(MergeOutcome::Conflicts(conflicts));
        }

        if self
            .run_git(&base_workspace, &["rev-parse", "--verify", "MERGE_HEAD"])
            .await
            .is_ok()
        {
            // Git allows unrelated local edits during merge, but the merge
            // commit must only contain session-branch changes.
            self.unstage_pre_merge_dirty_paths(&base_workspace, &pre_merge_dirty_paths)
                .await?;
            let message = commit_message.unwrap_or("Merge OpenTeams session changes");
            self.run_git(&base_workspace, &["commit", "-m", message])
                .await?;
        }

        Ok(MergeOutcome::Success)
    }

    /// Apply the net tree change since the last integrated session commit as
    /// one three-way patch. Unlike sequential cherry-picks, this surfaces all
    /// conflicts in a single batch and creates at most one normal commit.
    async fn execute_squash_merge(
        &self,
        row: &SessionWorktree,
        commit_message: Option<&str>,
    ) -> Result<MergeOutcome, SessionWorktreeError> {
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        let cursor = match row.base_commit.as_deref() {
            Some(value) => value.to_string(),
            None => self
                .run_git(
                    &base_workspace,
                    &[
                        "merge-base",
                        row.base_branch.as_str(),
                        row.branch_name.as_str(),
                    ],
                )
                .await?
                .trim()
                .to_string(),
        };
        let session_tip = self
            .run_git(&base_workspace, &["rev-parse", row.branch_name.as_str()])
            .await?
            .trim()
            .to_string();
        let patch_path =
            std::env::temp_dir().join(format!("openteams-session-squash-{}.patch", Uuid::new_v4()));
        let output_arg = format!("--output={}", patch_path.to_string_lossy());
        let diff_result = self
            .run_git(
                &base_workspace,
                &[
                    "diff",
                    "--binary",
                    "--full-index",
                    "--find-renames",
                    cursor.as_str(),
                    session_tip.as_str(),
                    output_arg.as_str(),
                ],
            )
            .await;
        if let Err(err) = diff_result {
            let _ = tokio::fs::remove_file(&patch_path).await;
            return Err(err);
        }

        let patch_is_empty = match tokio::fs::metadata(&patch_path).await {
            Ok(metadata) => metadata.len() == 0,
            Err(err) => {
                let _ = tokio::fs::remove_file(&patch_path).await;
                return Err(err.into());
            }
        };
        if patch_is_empty {
            let _ = tokio::fs::remove_file(&patch_path).await;
            return Ok(MergeOutcome::Success);
        }

        // Keep unrelated base-workspace edits out of the squash commit. This
        // preserves their working-tree content while making the integration
        // index contain only session changes.
        let pre_merge_dirty_paths = match self.dirty_tracked_paths(&base_workspace).await {
            Ok(paths) => paths,
            Err(err) => {
                let _ = tokio::fs::remove_file(&patch_path).await;
                return Err(err);
            }
        };
        if let Err(err) = self
            .unstage_pre_merge_dirty_paths(&base_workspace, &pre_merge_dirty_paths)
            .await
        {
            let _ = tokio::fs::remove_file(&patch_path).await;
            return Err(err);
        }

        let patch_arg = patch_path.to_string_lossy().to_string();
        let apply_result = self
            .run_git(
                &base_workspace,
                &["apply", "--3way", "--index", patch_arg.as_str()],
            )
            .await;
        let _ = tokio::fs::remove_file(&patch_path).await;

        if let Err(err) = apply_result {
            let conflicts = self.list_unmerged_paths(&base_workspace).await?;
            if !conflicts.is_empty() {
                return Ok(MergeOutcome::Conflicts(conflicts));
            }
            let _ = self
                .run_git(&base_workspace, &["reset", "--merge", "HEAD"])
                .await;
            return Err(err);
        }

        let staged_paths = match self
            .run_git_nul_paths(&base_workspace, &["diff", "--cached", "--name-only", "-z"])
            .await
        {
            Ok(paths) => paths,
            Err(err) => {
                let _ = self
                    .run_git(&base_workspace, &["reset", "--merge", "HEAD"])
                    .await;
                return Err(err);
            }
        };
        if !staged_paths.is_empty() {
            let message = commit_message
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("Merge OpenTeams session changes");
            if let Err(err) = self
                .run_git(&base_workspace, &["commit", "-m", message])
                .await
            {
                let _ = self
                    .run_git(&base_workspace, &["reset", "--merge", "HEAD"])
                    .await;
                return Err(err);
            }
        }

        Ok(MergeOutcome::Success)
    }

    /// Apply commits after the worktree's integration cursor. The cursor is
    /// advanced after a successful run so repeat merges only apply commits
    /// created since the previous integration.
    async fn execute_cherry_pick(
        &self,
        row: &SessionWorktree,
    ) -> Result<MergeOutcome, SessionWorktreeError> {
        self.execute_cherry_pick_after(row, row.base_commit.as_deref())
            .await
    }

    async fn execute_cherry_pick_after(
        &self,
        row: &SessionWorktree,
        cursor: Option<&str>,
    ) -> Result<MergeOutcome, SessionWorktreeError> {
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        for commit in self.pending_cherry_pick_commits(row, cursor).await? {
            let result = self
                .run_git(&base_workspace, &["cherry-pick", commit.as_str()])
                .await;
            if let Err(err) = result {
                let conflicts = self.list_unmerged_paths(&base_workspace).await?;
                if !conflicts.is_empty() {
                    return Ok(MergeOutcome::Conflicts(conflicts));
                }
                return Err(err);
            }
        }
        Ok(MergeOutcome::Success)
    }

    async fn pending_cherry_pick_commits(
        &self,
        row: &SessionWorktree,
        cursor: Option<&str>,
    ) -> Result<Vec<String>, SessionWorktreeError> {
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        let cursor = match cursor {
            Some(value) => value.to_string(),
            None => self
                .run_git(
                    &base_workspace,
                    &[
                        "merge-base",
                        row.base_branch.as_str(),
                        row.branch_name.as_str(),
                    ],
                )
                .await?
                .trim()
                .to_string(),
        };
        let range = format!("{cursor}..{}", row.branch_name);
        let output = self
            .run_git(&base_workspace, &["rev-list", "--reverse", range.as_str()])
            .await?;
        Ok(output
            .lines()
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    async fn mark_cursor_integration_merged(
        &self,
        session_id: Uuid,
        row: &SessionWorktree,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        let integrated_tip = self
            .run_git(&base_workspace, &["rev-parse", row.branch_name.as_str()])
            .await?
            .trim()
            .to_string();
        let merged = self.mark_merged(session_id).await?;
        SessionWorktree::record_base_commit(&self.pool, merged.id, Some(&integrated_tip))
            .await
            .map_err(Into::into)
    }

    /// List conflicted (unmerged) files for a session whose worktree is in
    /// `needs_conflict_resolution`. Returns one `ConflictFileInfo` per path.
    pub async fn list_conflict_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<ConflictFileInfo>, SessionWorktreeError> {
        let row = self.require_conflict_resolution(session_id).await?;
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        let status = self
            .run_git(
                &base_workspace,
                &["status", "--porcelain", "--untracked-files=no"],
            )
            .await?;
        Ok(parse_unmerged_porcelain(&status))
    }

    /// Read the three-way content for a single conflicted file from Git
    /// index stages. Prefers index stages over conflict markers so the UI
    /// can always recover the original sides even if the working-tree file
    /// has been edited.
    pub async fn read_conflict_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<ConflictFileContent, SessionWorktreeError> {
        validate_conflict_path(path)?;

        let row = self.require_conflict_resolution(session_id).await?;
        let base_workspace = PathBuf::from(&row.base_workspace_path);

        let base_stage = self.read_git_stage_bytes(&base_workspace, 1, path).await;
        let current_stage = self.read_git_stage_bytes(&base_workspace, 2, path).await;
        let session_stage = self.read_git_stage_bytes(&base_workspace, 3, path).await;

        let working_tree_bytes = tokio::fs::read(base_workspace.join(path))
            .await
            .unwrap_or_default();
        let size_bytes = working_tree_bytes.len() as u64;
        let working_tree_part = conflict_content_part(Some(&working_tree_bytes));
        let base_part = conflict_content_part(base_stage.as_deref());
        let current_part = conflict_content_part(current_stage.as_deref());
        let session_part = conflict_content_part(session_stage.as_deref());
        let is_binary = [
            working_tree_part.is_binary,
            base_part.is_binary,
            current_part.is_binary,
            session_part.is_binary,
        ]
        .into_iter()
        .any(|value| value);
        let is_too_large = [
            working_tree_part.is_too_large,
            base_part.is_too_large,
            current_part.is_too_large,
            session_part.is_too_large,
        ]
        .into_iter()
        .any(|value| value);

        Ok(ConflictFileContent {
            path: path.to_string(),
            base: base_part.text,
            current: current_part.text,
            session: session_part.text,
            working_tree: working_tree_part.text.unwrap_or_default(),
            is_binary,
            is_too_large,
            size_bytes,
        })
    }

    /// Write the user-resolved content for a conflicted file and `git add`
    /// it to mark the path as resolved.
    pub async fn resolve_conflict_file(
        &self,
        session_id: Uuid,
        path: &str,
        content: Option<&str>,
        use_stage: Option<ConflictResolutionSide>,
        delete_file: bool,
    ) -> Result<(), SessionWorktreeError> {
        validate_conflict_path(path)?;

        let row = self.require_conflict_resolution(session_id).await?;
        let base_workspace = PathBuf::from(&row.base_workspace_path);

        if delete_file {
            let _ = tokio::fs::remove_file(base_workspace.join(path)).await;
            self.run_git(
                &base_workspace,
                &["rm", "-f", "--ignore-unmatch", "--", path],
            )
            .await?;
            return Ok(());
        }

        if let Some(side) = use_stage {
            let checkout_arg = match side {
                ConflictResolutionSide::Current => "--ours",
                ConflictResolutionSide::Session => "--theirs",
            };
            self.run_git(&base_workspace, &["checkout", checkout_arg, "--", path])
                .await?;
            self.run_git(&base_workspace, &["add", "--", path]).await?;
            return Ok(());
        }

        let file_path = base_workspace.join(path);
        if let Some(parent) = file_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(&file_path, content.unwrap_or_default()).await?;

        self.run_git(&base_workspace, &["add", "--", path]).await?;
        Ok(())
    }

    /// Complete an integration after all conflicts have been resolved.
    /// Cursor-based strategies create normal single-parent commits; legacy
    /// merge strategies complete their pending merge commit.
    pub async fn continue_merge(
        &self,
        session_id: Uuid,
        commit_message: Option<String>,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = self.require_conflict_resolution(session_id).await?;
        let base_workspace = PathBuf::from(&row.base_workspace_path);

        let unmerged = self.list_unmerged_paths(&base_workspace).await?;
        if !unmerged.is_empty() {
            return Err(SessionWorktreeError::UnresolvedConflicts(unmerged));
        }

        if row.merge_operation == Some(SessionWorktreeMergeOperation::CherryPick) {
            let cherry_pick_head = self
                .run_git(&base_workspace, &["rev-parse", "CHERRY_PICK_HEAD"])
                .await?
                .trim()
                .to_string();
            let staged_paths = self
                .run_git_nul_paths(&base_workspace, &["diff", "--cached", "--name-only", "-z"])
                .await?;
            if staged_paths.is_empty() {
                // Resolving every conflict to the current/base side can make
                // the cherry-pick empty. Skip it instead of manufacturing an
                // empty replacement commit.
                self.run_git(&base_workspace, &["cherry-pick", "--skip"])
                    .await?;
            } else {
                // Avoid opening an editor while retaining the original commit
                // message recorded by CHERRY_PICK_HEAD.
                self.run_git(
                    &base_workspace,
                    &["-c", "core.editor=true", "cherry-pick", "--continue"],
                )
                .await?;
                if let Some(message) = commit_message
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                {
                    self.run_git(&base_workspace, &["commit", "--amend", "-m", message])
                        .await?;
                }
            }

            return match self
                .execute_cherry_pick_after(&row, Some(&cherry_pick_head))
                .await?
            {
                MergeOutcome::Success => {
                    self.mark_cursor_integration_merged(session_id, &row).await
                }
                MergeOutcome::Conflicts(files) => {
                    SessionWorktree::set_conflict_files(&self.pool, row.id, &files)
                        .await
                        .map_err(Into::into)
                }
            };
        }

        if row.merge_operation == Some(SessionWorktreeMergeOperation::SquashMerge) {
            let staged_paths = self
                .run_git_nul_paths(&base_workspace, &["diff", "--cached", "--name-only", "-z"])
                .await?;
            if !staged_paths.is_empty() {
                let message = commit_message
                    .as_deref()
                    .filter(|value| !value.trim().is_empty())
                    .unwrap_or("Merge OpenTeams session changes");
                self.run_git(&base_workspace, &["commit", "-m", message])
                    .await?;
            }
            return self.mark_cursor_integration_merged(session_id, &row).await;
        }

        let message = commit_message
            .as_deref()
            .unwrap_or("Merge OpenTeams session changes");
        self.run_git(&base_workspace, &["commit", "-m", message])
            .await?;

        self.mark_merged(session_id).await
    }

    /// Abort an in-progress merge: undo the Git merge state in the
    /// base workspace and transition the worktree back to `dirty`. This is
    /// the route-facing abort that combines Git cleanup with state
    /// transition.
    pub async fn perform_abort_merge(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;

        if row.status != SessionWorktreeStatus::NeedsConflictResolution
            && row.status != SessionWorktreeStatus::Merging
        {
            return Err(SessionWorktreeError::NoMergeInProgress(session_id));
        }

        let base_workspace = PathBuf::from(&row.base_workspace_path);
        // Abort the operation that was persisted when the integration began.
        // Squash integration has no MERGE_HEAD, so reset its index/worktree
        // changes while preserving unrelated local edits.
        if row.merge_operation == Some(SessionWorktreeMergeOperation::SquashMerge) {
            self.run_git(&base_workspace, &["reset", "--merge", "HEAD"])
                .await?;
        } else {
            let abort_args =
                if row.merge_operation == Some(SessionWorktreeMergeOperation::CherryPick) {
                    ["cherry-pick", "--abort"]
                } else {
                    ["merge", "--abort"]
                };
            let _ = self.run_git(&base_workspace, &abort_args).await;
            let _ = self
                .run_git(&base_workspace, &["reset", "--hard", "HEAD"])
                .await;
        }

        self.abort_merge(session_id, false).await
    }

    // -----------------------------------------------------------------
    // Private Git helpers
    // -----------------------------------------------------------------

    async fn require_conflict_resolution(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?;
        if row.status != SessionWorktreeStatus::NeedsConflictResolution {
            return Err(SessionWorktreeError::NoMergeInProgress(session_id));
        }
        Ok(row)
    }

    async fn run_git(
        &self,
        repo_path: &Path,
        args: &[&str],
    ) -> Result<String, SessionWorktreeError> {
        let stdout = self.run_git_bytes(repo_path, args).await?;
        Ok(String::from_utf8_lossy(&stdout).to_string())
    }

    async fn run_git_bytes(
        &self,
        repo_path: &Path,
        args: &[&str],
    ) -> Result<Vec<u8>, SessionWorktreeError> {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(repo_path)
            .args(args)
            .kill_on_drop(true);
        let output = timeout(GIT_COMMAND_TIMEOUT, command.output())
            .await
            .map_err(|_| {
                SessionWorktreeError::GitCommand(format!(
                    "git {} timed out after {}s",
                    args.join(" "),
                    GIT_COMMAND_TIMEOUT.as_secs()
                ))
            })?
            .map_err(|e| SessionWorktreeError::GitCommand(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let detail = if stderr.is_empty() { stdout } else { stderr };
            return Err(SessionWorktreeError::GitCommand(detail));
        }

        Ok(output.stdout)
    }

    async fn branch_has_commits_not_in_base(
        &self,
        row: &SessionWorktree,
    ) -> Result<bool, SessionWorktreeError> {
        let base_workspace = PathBuf::from(&row.base_workspace_path);
        if self
            .run_git(
                &base_workspace,
                &["rev-parse", "--verify", row.branch_name.as_str()],
            )
            .await
            .is_err()
        {
            return Ok(false);
        }

        match row.merge_operation {
            Some(SessionWorktreeMergeOperation::CherryPick) => {
                return Ok(!self
                    .pending_cherry_pick_commits(row, row.base_commit.as_deref())
                    .await?
                    .is_empty());
            }
            Some(SessionWorktreeMergeOperation::SquashMerge) => {
                let cursor = match row.base_commit.as_deref() {
                    Some(value) => value.to_string(),
                    None => self
                        .run_git(
                            &base_workspace,
                            &[
                                "merge-base",
                                row.base_branch.as_str(),
                                row.branch_name.as_str(),
                            ],
                        )
                        .await?
                        .trim()
                        .to_string(),
                };
                let changed_paths = self
                    .run_git(
                        &base_workspace,
                        &[
                            "diff",
                            "--name-only",
                            cursor.as_str(),
                            row.branch_name.as_str(),
                        ],
                    )
                    .await?;
                return Ok(!changed_paths.trim().is_empty());
            }
            _ => {}
        }

        let output = Command::new("git")
            .arg("-C")
            .arg(&base_workspace)
            .args([
                "merge-base",
                "--is-ancestor",
                row.branch_name.as_str(),
                row.base_branch.as_str(),
            ])
            .output()
            .await
            .map_err(|e| SessionWorktreeError::GitCommand(e.to_string()))?;

        match output.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
                let detail = if stderr.is_empty() { stdout } else { stderr };
                Err(SessionWorktreeError::GitCommand(detail))
            }
        }
    }

    async fn current_branch(
        &self,
        repo_path: &Path,
    ) -> Result<Option<String>, SessionWorktreeError> {
        let output = self
            .run_git(repo_path, &["branch", "--show-current"])
            .await?;
        let branch = output.trim().to_string();
        if branch.is_empty() {
            Ok(None)
        } else {
            Ok(Some(branch))
        }
    }

    async fn dirty_tracked_paths(
        &self,
        repo_path: &Path,
    ) -> Result<Vec<String>, SessionWorktreeError> {
        let mut paths = Vec::new();
        for args in [
            ["diff", "--name-only", "-z"].as_slice(),
            ["diff", "--cached", "--name-only", "-z"].as_slice(),
        ] {
            for path in self.run_git_nul_paths(repo_path, args).await? {
                if !paths.contains(&path) {
                    paths.push(path);
                }
            }
        }
        Ok(paths)
    }

    async fn run_git_nul_paths(
        &self,
        repo_path: &Path,
        args: &[&str],
    ) -> Result<Vec<String>, SessionWorktreeError> {
        let output = self.run_git_bytes(repo_path, args).await?;
        Ok(output
            .split(|byte| *byte == 0)
            .filter(|part| !part.is_empty())
            .map(|part| String::from_utf8_lossy(part).to_string())
            .collect())
    }

    async fn unstage_pre_merge_dirty_paths(
        &self,
        repo_path: &Path,
        paths: &[String],
    ) -> Result<(), SessionWorktreeError> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut args = vec!["reset", "-q", "HEAD", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run_git(repo_path, &args).await.map(|_| ())
    }

    async fn has_operation_in_progress(
        &self,
        repo_path: &Path,
    ) -> Result<bool, SessionWorktreeError> {
        for marker in &["MERGE_HEAD", "CHERRY_PICK_HEAD", "REVERT_HEAD"] {
            if self
                .run_git(repo_path, &["rev-parse", "--verify", marker])
                .await
                .is_ok()
            {
                return Ok(true);
            }
        }
        let git_dir = self
            .run_git(repo_path, &["rev-parse", "--git-dir"])
            .await?
            .trim()
            .to_string();
        let git_path = if git_dir.is_empty() {
            repo_path.join(".git")
        } else {
            repo_path.join(&git_dir)
        };
        if git_path.join("rebase-merge").exists() || git_path.join("rebase-apply").exists() {
            return Ok(true);
        }
        Ok(!self.list_unmerged_paths(repo_path).await?.is_empty())
    }

    async fn list_unmerged_paths(
        &self,
        repo_path: &Path,
    ) -> Result<Vec<String>, SessionWorktreeError> {
        let output = self
            .run_git(repo_path, &["diff", "--name-only", "--diff-filter=U"])
            .await?;
        Ok(output
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect())
    }

    async fn read_git_stage_bytes(
        &self,
        repo_path: &Path,
        stage: u8,
        path: &str,
    ) -> Option<Vec<u8>> {
        let stage_ref = format!(":{stage}:{path}");
        self.run_git_bytes(repo_path, &["show", &stage_ref])
            .await
            .ok()
    }

    // -----------------------------------------------------------------
    // (existing cleanup methods follow)
    // -----------------------------------------------------------------

    /// Force-remove every isolated workspace associated with a session before
    /// the session row is deleted. Session deletion is explicit destructive
    /// intent, so this path is allowed to remove worktrees in any lifecycle
    /// state, including worktrees with unmerged changes.
    ///
    /// The worktree rows are intentionally left untouched: the immediately
    /// following session deletion removes them through the foreign-key
    /// cascade. Keeping filesystem cleanup ahead of that cascade preserves the
    /// metadata needed to remove both the Git worktree registration and the
    /// session branch.
    pub async fn force_cleanup_for_session_deletion(
        &self,
        session_id: Uuid,
    ) -> Result<(), SessionWorktreeError> {
        let rows = SessionWorktree::find_all_by_session(&self.pool, session_id).await?;
        let mut cleaned_worktrees = HashSet::new();

        for row in rows.iter().rev() {
            let cleanup_key = (row.repo_path.clone(), row.worktree_path.clone());
            if cleaned_worktrees.insert(cleanup_key) {
                let cleanup = WorktreeCleanup::new(
                    PathBuf::from(&row.worktree_path),
                    Some(PathBuf::from(&row.repo_path)),
                );
                WorktreeManager::force_remove_worktree(&cleanup).await?;
            }

            self.delete_session_branch_if_present(row).await?;
        }

        Ok(())
    }

    ///
    /// Along with `force_cleanup_for_session_deletion`, this is the only path
    /// that may remove an `active` / `dirty` / `merging` /
    /// `needs_conflict_resolution` worktree. The cleanup is always
    /// authoritative because the user has explicitly confirmed the discard
    /// through the UI.
    ///
    /// Flow: row -> `cleanup_pending` -> `WorktreeManager::cleanup_worktree`
    /// (force) -> `archived`. On cleanup failure the row lands in
    /// `cleanup_failed` and the user may retry via `retry_cleanup`.
    pub async fn discard_worktree(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        let row = match SessionWorktree::find_active_by_session(&self.pool, session_id).await? {
            Some(row) => row,
            None => SessionWorktree::find_latest_by_session_and_status(
                &self.pool,
                session_id,
                SessionWorktreeStatus::Merged,
            )
            .await?
            .ok_or(SessionWorktreeError::NoActiveWorktree(session_id))?,
        };

        validate_transition(row.status, SessionWorktreeStatus::CleanupPending).map_err(|to| {
            SessionWorktreeError::IllegalTransition {
                session_id,
                from: row.status,
                to,
            }
        })?;

        let pending = Self::apply_transition(
            &self.pool,
            row.id,
            session_id,
            row.status,
            SessionWorktreeStatus::CleanupPending,
        )
        .await?;

        self.run_cleanup(pending).await
    }

    /// Disabled legacy cleanup path for worktrees that have already been merged.
    /// A successful merge preserves the physical worktree path; callers must
    /// whose status is not `merged` — unmerged worktrees require an explicit
    /// use `discard_worktree` to remove it after merge.
    ///
    pub async fn cleanup_merged_worktree(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        Err(SessionWorktreeError::MergedCleanupRequiresDiscard(
            session_id,
        ))
    }

    /// Retry a previously failed cleanup. Transitions `cleanup_failed` back
    /// to `cleanup_pending` and re-runs the worktree removal. If the worktree
    /// directory is already gone (e.g. removed out-of-band), the cleanup is
    /// treated as successful.
    ///
    /// Looks up the row via `find_latest_by_session_and_status(_, _,
    /// CleanupFailed)` because `find_active_by_session` deliberately excludes
    /// `cleanup_failed` rows.
    ///
    /// **Path-collision safety guard**: same as `cleanup_merged_worktree` —
    /// `worktree_path_for_session` is derived only from the session id, so a
    /// historical `cleanup_failed` row and a newer `active` row for the same
    /// session share the same physical path. Removing the historical row's
    /// path would delete the active worktree. We refuse if the session has
    /// any non-terminal row, forcing the caller to resolve the active
    /// worktree first.
    pub async fn retry_cleanup(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        self.refuse_if_session_has_active(session_id).await?;

        let row = SessionWorktree::find_latest_by_session_and_status(
            &self.pool,
            session_id,
            SessionWorktreeStatus::CleanupFailed,
        )
        .await?
        .ok_or(SessionWorktreeError::NoCleanupFailedWorktree(session_id))?;

        let pending = Self::apply_transition(
            &self.pool,
            row.id,
            session_id,
            SessionWorktreeStatus::CleanupFailed,
            SessionWorktreeStatus::CleanupPending,
        )
        .await?;

        self.run_cleanup(pending).await
    }

    /// Force-remove a previously failed cleanup row. This is intentionally
    /// only available after a normal cleanup attempt has failed, and is meant
    /// for Windows process-lock failures where Git metadata can be detached
    /// even though the physical directory cannot be deleted immediately.
    pub async fn force_remove_failed_cleanup(
        &self,
        session_id: Uuid,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        self.refuse_if_session_has_active(session_id).await?;

        let row = SessionWorktree::find_latest_by_session_and_status(
            &self.pool,
            session_id,
            SessionWorktreeStatus::CleanupFailed,
        )
        .await?
        .ok_or(SessionWorktreeError::NoCleanupFailedWorktree(session_id))?;

        let pending = Self::apply_transition(
            &self.pool,
            row.id,
            session_id,
            SessionWorktreeStatus::CleanupFailed,
            SessionWorktreeStatus::CleanupPending,
        )
        .await?;

        self.run_force_cleanup(pending).await
    }

    /// Safety guard for `cleanup_merged_worktree` and `retry_cleanup`: refuse
    /// to remove a historical terminal worktree's path when the session still
    /// has a non-terminal worktree row. `worktree_path_for_session` is derived
    /// only from the session id, so a historical and a current worktree for
    /// the same session share the same physical path — removing the old row
    /// would delete the active worktree. The caller must resolve the active
    /// row first (via `discard_worktree` or by letting it reach a terminal
    /// state) before this guard passes.
    async fn refuse_if_session_has_active(
        &self,
        session_id: Uuid,
    ) -> Result<(), SessionWorktreeError> {
        if SessionWorktree::find_active_by_session(&self.pool, session_id)
            .await?
            .is_some()
        {
            return Err(SessionWorktreeError::SessionHasActiveWorktree(session_id));
        }
        Ok(())
    }

    /// Execute `git worktree remove` + metadata cleanup for a row already in
    /// `cleanup_pending`, then transition to `archived` (success) or
    /// `cleanup_failed` (error). `WorktreeManager::cleanup_worktree` already
    /// prefers `git worktree remove` and falls back to filesystem +
    /// metadata cleanup, satisfying the design's "delete must prefer git
    /// worktree remove" requirement.
    async fn run_cleanup(
        &self,
        row: SessionWorktree,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        debug_assert_eq!(row.status, SessionWorktreeStatus::CleanupPending);

        let cleanup = WorktreeCleanup::new(
            PathBuf::from(&row.worktree_path),
            Some(PathBuf::from(&row.repo_path)),
        );
        match WorktreeManager::cleanup_worktree(&cleanup).await {
            Ok(()) => {
                self.delete_session_branch_if_present(&row).await?;
                // Use the clearing variant for the success terminal transition
                // so stale `cleanup_error` from a prior failed attempt (or
                // leftover merge metadata from a discarded merge) does not
                // leak into the archived audit row.
                let archived = Self::apply_transition_clearing_transient(
                    &self.pool,
                    row.id,
                    row.session_id,
                    SessionWorktreeStatus::CleanupPending,
                    SessionWorktreeStatus::Archived,
                )
                .await?;
                // Re-read via record_archived_at so the returned row carries
                // the freshly-stamped archived_at timestamp.
                let archived = SessionWorktree::record_archived_at(&self.pool, archived.id).await?;
                self.sync_session_agent_workspace_paths(
                    archived.session_id,
                    &archived.base_workspace_path,
                )
                .await?;
                info!(
                    worktree_id = %archived.id,
                    session_id = %archived.session_id,
                    "Cleaned up session worktree"
                );
                Ok(archived)
            }
            Err(err) => {
                let message = err.to_string();
                warn!(
                    worktree_id = %row.id,
                    session_id = %row.session_id,
                    error = %message,
                    "Session worktree cleanup failed"
                );
                let failed = SessionWorktree::transition_status_clearing_transient(
                    &self.pool,
                    row.id,
                    SessionWorktreeStatus::CleanupPending,
                    SessionWorktreeStatus::CleanupFailed,
                )
                .await?;
                let _ = SessionWorktree::set_cleanup_error(&self.pool, failed.id, &message).await;
                let failed = SessionWorktree::find_by_id(&self.pool, failed.id)
                    .await?
                    .unwrap_or(failed);
                InboxService::new()
                    .notify_worktree_cleanup_failed(&self.pool, &failed, &message)
                    .await;
                Err(SessionWorktreeError::WorktreeManager(err))
            }
        }
    }

    async fn run_force_cleanup(
        &self,
        row: SessionWorktree,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        debug_assert_eq!(row.status, SessionWorktreeStatus::CleanupPending);

        let cleanup = WorktreeCleanup::new(
            PathBuf::from(&row.worktree_path),
            Some(PathBuf::from(&row.repo_path)),
        );
        match WorktreeManager::force_remove_worktree(&cleanup).await {
            Ok(()) => {
                self.delete_session_branch_if_present(&row).await?;
                let archived = Self::apply_transition_clearing_transient(
                    &self.pool,
                    row.id,
                    row.session_id,
                    SessionWorktreeStatus::CleanupPending,
                    SessionWorktreeStatus::Archived,
                )
                .await?;
                let archived = SessionWorktree::record_archived_at(&self.pool, archived.id).await?;
                self.sync_session_agent_workspace_paths(
                    archived.session_id,
                    &archived.base_workspace_path,
                )
                .await?;
                info!(
                    worktree_id = %archived.id,
                    session_id = %archived.session_id,
                    "Force-removed session worktree"
                );
                Ok(archived)
            }
            Err(err) => {
                let message = err.to_string();
                warn!(
                    worktree_id = %row.id,
                    session_id = %row.session_id,
                    error = %message,
                    "Session worktree force remove failed"
                );
                let failed = SessionWorktree::transition_status_clearing_transient(
                    &self.pool,
                    row.id,
                    SessionWorktreeStatus::CleanupPending,
                    SessionWorktreeStatus::CleanupFailed,
                )
                .await?;
                let _ = SessionWorktree::set_cleanup_error(&self.pool, failed.id, &message).await;
                let failed = SessionWorktree::find_by_id(&self.pool, failed.id)
                    .await?
                    .unwrap_or(failed);
                InboxService::new()
                    .notify_worktree_cleanup_failed(&self.pool, &failed, &message)
                    .await;
                Err(SessionWorktreeError::WorktreeManager(err))
            }
        }
    }

    async fn delete_session_branch_if_present(
        &self,
        row: &SessionWorktree,
    ) -> Result<(), SessionWorktreeError> {
        let repo_path = PathBuf::from(&row.repo_path);
        if detect_git_repo_path(&repo_path).is_none() {
            debug!(
                worktree_id = %row.id,
                session_id = %row.session_id,
                repo_path = %repo_path.display(),
                "Skipping session branch deletion because repo path is not a git repository"
            );
            return Ok(());
        }

        match self
            .run_git(&repo_path, &["branch", "-D", &row.branch_name])
            .await
        {
            Ok(_) => {
                info!(
                    worktree_id = %row.id,
                    session_id = %row.session_id,
                    branch = %row.branch_name,
                    "Deleted session worktree branch"
                );
                Ok(())
            }
            Err(SessionWorktreeError::GitCommand(message))
                if message.contains("not found")
                    || message.contains("branch not found")
                    || message.contains("not a branch") =>
            {
                debug!(
                    worktree_id = %row.id,
                    session_id = %row.session_id,
                    branch = %row.branch_name,
                    "Session worktree branch already absent"
                );
                Ok(())
            }
            Err(err) => Err(err),
        }
    }

    /// Validate then CAS-apply a transition. The validation step is the
    /// authoritative gate; the CAS defends against a concurrent writer that
    /// raced between our read of `from` and the UPDATE. Either failure mode
    /// surfaces as `IllegalTransition` so callers see one error kind.
    async fn apply_transition(
        pool: &Pool<Sqlite>,
        worktree_id: Uuid,
        session_id: Uuid,
        from: SessionWorktreeStatus,
        to: SessionWorktreeStatus,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        Self::apply_transition_inner(pool, worktree_id, session_id, from, to, false).await
    }

    /// Same as `apply_transition` but also clears transient merge / cleanup
    /// metadata (merge_operation, merge_target_branch, conflict_files_json,
    /// operation_started_at, cleanup_error). Use when leaving a state that
    /// carries such metadata so the next lifecycle starts from a clean slate
    /// — e.g. cleanup_pending -> archived on a successful retry must not
    /// retain the cleanup_error from the prior failure.
    async fn apply_transition_clearing_transient(
        pool: &Pool<Sqlite>,
        worktree_id: Uuid,
        session_id: Uuid,
        from: SessionWorktreeStatus,
        to: SessionWorktreeStatus,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        Self::apply_transition_inner(pool, worktree_id, session_id, from, to, true).await
    }

    async fn apply_transition_inner(
        pool: &Pool<Sqlite>,
        worktree_id: Uuid,
        session_id: Uuid,
        from: SessionWorktreeStatus,
        to: SessionWorktreeStatus,
        clear_transient: bool,
    ) -> Result<SessionWorktree, SessionWorktreeError> {
        validate_transition(from, to).map_err(|to| SessionWorktreeError::IllegalTransition {
            session_id,
            from,
            to,
        })?;
        let result = if clear_transient {
            SessionWorktree::transition_status_clearing_transient(pool, worktree_id, from, to).await
        } else {
            SessionWorktree::transition_status(pool, worktree_id, from, to).await
        };
        result.map_err(|err| match err {
            ModelCasError::CasRejected { .. } => SessionWorktreeError::IllegalTransition {
                session_id,
                from,
                to,
            },
            other => other.into(),
        })
    }
}

#[cfg(test)]
mod tests;

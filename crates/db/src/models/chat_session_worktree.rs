use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool, Type};
use thiserror::Error;
use ts_rs::TS;
use uuid::Uuid;

const CHAT_SESSION_WORKTREE_SELECT: &str = r#"
    SELECT id,
           session_id,
           project_id,
           base_workspace_path,
           repo_path,
           base_branch,
           base_commit,
           branch_name,
           worktree_path,
           mode,
           status,
           0 AS has_unmerged_commits,
           merge_target_branch,
           merge_operation,
           conflict_files_json,
           operation_started_at,
           cleanup_error,
           last_used_at,
           merged_at,
           archived_at,
           created_at,
           updated_at
    FROM chat_session_worktrees
"#;

const CHAT_SESSION_WORKTREE_RETURNING: &str = r#"
    RETURNING id,
              session_id,
              project_id,
              base_workspace_path,
              repo_path,
              base_branch,
              base_commit,
              branch_name,
              worktree_path,
              mode,
              status,
              0 AS has_unmerged_commits,
              merge_target_branch,
              merge_operation,
              conflict_files_json,
              operation_started_at,
              cleanup_error,
              last_used_at,
              merged_at,
              archived_at,
              created_at,
              updated_at
"#;

/// Lifecycle of a session worktree.
///
/// Only the `SessionWorktreeService` (the reducer) may write this column.
/// Wire format is `snake_case`; never serialize via `Debug` lowercasing.
///
/// Transitions (authoritative list lives in the service):
/// - `creating` -> `active` | `cleanup_failed`
/// - `active`   -> `dirty` | `merging` | `cleanup_pending` | `archived`
/// - `dirty`    -> `active` | `merging` | `cleanup_pending` | `archived`
/// - `merging`  -> `merged` | `needs_conflict_resolution` | `dirty` | `active` | `cleanup_pending`
/// - `needs_conflict_resolution` -> `merged` | `dirty` | `merging`
/// - `merged`   -> `dirty` | `cleanup_pending` | `archived`
/// - `cleanup_pending` -> `archived` | `cleanup_failed`
/// - `cleanup_failed`  -> `cleanup_pending` | `archived`
/// - `archived` is terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "session_worktree_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[ts(use_ts_enum)]
pub enum SessionWorktreeStatus {
    Creating,
    Active,
    Dirty,
    Merging,
    NeedsConflictResolution,
    Merged,
    Archived,
    CleanupPending,
    CleanupFailed,
}

impl SessionWorktreeStatus {
    /// States considered terminal for runtime scheduling. Rows in these states
    /// no longer participate in agent runs or source-control; they are kept
    /// for audit and (for `archived`) replay only.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Archived)
    }

    /// States where the worktree path is the active workspace for agent runs
    /// and source-control. `cleanup_pending` is intentionally excluded even
    /// though it is still returned by `find_active_by_session` for lifecycle
    /// guards: the physical worktree may already be removed during cleanup, so
    /// new runs must fall back to the base workspace instead of recreating an
    /// empty directory.
    ///
    /// `merged` remains active until the user explicitly discards it. The
    /// terminal/cleanup states (`cleanup_pending`, `archived`,
    /// `cleanup_failed`) return `false` — the caller should switch to the
    /// worktree row's `base_workspace_path` instead.
    pub fn is_active_for_workspace(self) -> bool {
        matches!(
            self,
            Self::Creating
                | Self::Active
                | Self::Dirty
                | Self::Merging
                | Self::NeedsConflictResolution
                | Self::Merged
        )
    }

    /// States that block automatic cleanup. `active`, `dirty`, `merging`, and
    /// `needs_conflict_resolution` carry unmerged user changes; `merged` keeps
    /// the physical worktree available for audit until the user explicitly
    /// discards it.
    pub fn blocks_auto_cleanup(self) -> bool {
        matches!(
            self,
            Self::Active
                | Self::Dirty
                | Self::Merging
                | Self::NeedsConflictResolution
                | Self::Creating
                | Self::Merged
        )
    }
}

/// Authoritative `snake_case` wire format for `SessionWorktreeStatus`.
///
/// This is the typed `Display` callers MUST use instead of
/// `format!("{:?}", status).to_lowercase()` (which the AGENTS.md
/// workflow-pitfalls section explicitly forbids). The string produced here
/// is the single source of truth and matches the `serde`/`sqlx`
/// `rename_all = "snake_case"` mapping.
impl std::fmt::Display for SessionWorktreeStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Creating => "creating",
            Self::Active => "active",
            Self::Dirty => "dirty",
            Self::Merging => "merging",
            Self::NeedsConflictResolution => "needs_conflict_resolution",
            Self::Merged => "merged",
            Self::Archived => "archived",
            Self::CleanupPending => "cleanup_pending",
            Self::CleanupFailed => "cleanup_failed",
        };
        f.write_str(s)
    }
}

/// Phase 1 only supports one worktree per session. The enum is kept for
/// forward-compatibility with per-agent / per-step worktrees (Phase 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(type_name = "session_worktree_mode", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[ts(use_ts_enum)]
pub enum SessionWorktreeMode {
    Session,
}

impl std::fmt::Display for SessionWorktreeMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Session => f.write_str("session"),
        }
    }
}

/// Merge strategy recorded when entering `merging`. Persisted so the
/// `needs_conflict_resolution` UI and `merge/continue` can resume the
/// correct git operation after a refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Type, Serialize, Deserialize, TS)]
#[sqlx(
    type_name = "session_worktree_merge_operation",
    rename_all = "snake_case"
)]
#[serde(rename_all = "snake_case")]
#[ts(use_ts_enum)]
pub enum SessionWorktreeMergeOperation {
    Merge,
    SquashMerge,
    CherryPick,
    Rebase,
}

impl std::fmt::Display for SessionWorktreeMergeOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Merge => "merge",
            Self::SquashMerge => "squash_merge",
            Self::CherryPick => "cherry_pick",
            Self::Rebase => "rebase",
        };
        f.write_str(s)
    }
}

/// Error returned by compare-and-swap transitions when no row matched the
/// expected `from` status. The caller (service) is responsible for surfacing
/// this as a reducer rejection with `worktree_id`, `from`, `to` context.
#[derive(Debug, Error)]
pub enum SessionWorktreeError {
    #[error(transparent)]
    Database(#[from] sqlx::Error),
    #[error(
        "session worktree transition rejected: worktree_id={worktree_id} expected={expected:?} actual={actual:?}"
    )]
    CasRejected {
        worktree_id: Uuid,
        expected: SessionWorktreeStatus,
        actual: Option<SessionWorktreeStatus>,
    },
}

/// A registered session worktree.
///
/// `repo_path` points at the main repository (the source of `git worktree add`);
/// `worktree_path` is the isolated checkout agents run in. `base_workspace_path`
/// is the user-facing main workspace the worktree was branched from, kept so
/// the UI can label the merge target without re-resolving it.
#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct SessionWorktree {
    pub id: Uuid,
    pub session_id: Uuid,
    pub project_id: Option<Uuid>,
    pub base_workspace_path: String,
    pub repo_path: String,
    pub base_branch: String,
    pub base_commit: Option<String>,
    pub branch_name: String,
    pub worktree_path: String,
    pub mode: SessionWorktreeMode,
    pub status: SessionWorktreeStatus,
    /// Runtime-only merge availability derived from the branch relationship.
    /// This is selected as `false` from SQLite and populated by
    /// `SessionWorktreeService::get_latest_for_session` before returning the
    /// status read model to callers.
    pub has_unmerged_commits: bool,
    pub merge_target_branch: Option<String>,
    pub merge_operation: Option<SessionWorktreeMergeOperation>,
    /// JSON array of file paths (relative to the merge target) that are in
    /// conflict while `status = needs_conflict_resolution`. Stored as raw
    /// text so we do not couple the wire type to `sqlx::types::Json`.
    pub conflict_files_json: String,
    pub operation_started_at: Option<DateTime<Utc>>,
    pub cleanup_error: Option<String>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
    pub archived_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl SessionWorktree {
    /// Parse `conflict_files_json` into a sorted list of relative paths.
    /// Returns an empty vector on parse failure rather than propagating the
    /// error: conflict files are advisory UI state and a corrupt JSON blob
    /// must not block `merge/continue`.
    pub fn conflict_files(&self) -> Vec<String> {
        serde_json::from_str::<Vec<String>>(&self.conflict_files_json)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| !s.trim().is_empty())
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct CreateSessionWorktree {
    pub session_id: Uuid,
    pub project_id: Option<Uuid>,
    pub base_workspace_path: String,
    pub repo_path: String,
    pub base_branch: String,
    pub base_commit: Option<String>,
    pub branch_name: String,
    pub worktree_path: String,
    pub mode: SessionWorktreeMode,
}

impl SessionWorktree {
    pub async fn find_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!("{CHAT_SESSION_WORKTREE_SELECT}\nWHERE id = ?1"))
            .bind(id)
            .fetch_optional(pool)
            .await
    }

    /// All rows for a session, oldest first. Use this for audit/history views.
    pub async fn find_all_by_session(
        pool: &SqlitePool,
        session_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            "{CHAT_SESSION_WORKTREE_SELECT}\nWHERE session_id = ?1\nORDER BY created_at ASC"
        ))
        .bind(session_id)
        .fetch_all(pool)
        .await
    }

    /// The single active workspace row for a session, or `None`.
    ///
    /// Active workspace rows include `merged` because the physical worktree is
    /// preserved for follow-up commits and repeat merges. Archived/failed rows
    /// are excluded so a session can create a new worktree after cleanup.
    pub async fn find_active_by_session(
        pool: &SqlitePool,
        session_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            "{CHAT_SESSION_WORKTREE_SELECT}\n\
             WHERE session_id = ?1\n\
               AND status IN (\n\
                   'creating', 'active', 'dirty', 'merging',\n\
                   'needs_conflict_resolution', 'merged', 'cleanup_pending'\n\
               )\n\
             ORDER BY created_at DESC\n\
             LIMIT 1"
        ))
        .bind(session_id)
        .fetch_optional(pool)
        .await
    }

    /// Convenience for janitor / reconciliation passes: the most recent row
    /// for a session regardless of status.
    pub async fn find_latest_for_session(
        pool: &SqlitePool,
        session_id: Uuid,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            "{CHAT_SESSION_WORKTREE_SELECT}\n\
             WHERE session_id = ?1\n\
             ORDER BY created_at DESC\n\
             LIMIT 1"
        ))
        .bind(session_id)
        .fetch_optional(pool)
        .await
    }

    /// Most recent row for a session in the exact given status. Use this for
    /// lookups that target a specific terminal / non-terminal state, e.g.
    /// `discard_worktree` needs to find a `merged` row (which
    /// `find_active_by_session` deliberately excludes). Returns the latest
    /// match so a session with multiple historical rows of the same status
    /// (e.g. two `cleanup_failed` rows from different attempts) resolves to
    /// the most recent one.
    pub async fn find_latest_by_session_and_status(
        pool: &SqlitePool,
        session_id: Uuid,
        status: SessionWorktreeStatus,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            "{CHAT_SESSION_WORKTREE_SELECT}\n\
             WHERE session_id = ?1\n\
               AND status = ?2\n\
             ORDER BY created_at DESC\n\
             LIMIT 1"
        ))
        .bind(session_id)
        .bind(status)
        .fetch_optional(pool)
        .await
    }

    pub async fn find_by_worktree_path(
        pool: &SqlitePool,
        worktree_path: &str,
    ) -> Result<Option<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            "{CHAT_SESSION_WORKTREE_SELECT}\nWHERE worktree_path = ?1"
        ))
        .bind(worktree_path)
        .fetch_optional(pool)
        .await
    }

    /// Worktree rows whose merge operation can leave the base workspace in a
    /// Git operation-in-progress state. Source-control uses this to scope that
    /// state back to the owning isolated session.
    pub async fn find_merge_operations_by_project(
        pool: &SqlitePool,
        project_id: Uuid,
    ) -> Result<Vec<Self>, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            "{CHAT_SESSION_WORKTREE_SELECT}\n\
             WHERE project_id = ?1\n\
               AND status IN ('merging', 'needs_conflict_resolution')\n\
             ORDER BY updated_at DESC"
        ))
        .bind(project_id)
        .fetch_all(pool)
        .await
    }

    pub async fn create(
        pool: &SqlitePool,
        data: &CreateSessionWorktree,
        id: Uuid,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            INSERT INTO chat_session_worktrees (
                id, session_id, project_id, base_workspace_path, repo_path,
                base_branch, base_commit, branch_name, worktree_path, mode
            )
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(data.session_id)
        .bind(data.project_id)
        .bind(&data.base_workspace_path)
        .bind(&data.repo_path)
        .bind(&data.base_branch)
        .bind(&data.base_commit)
        .bind(&data.branch_name)
        .bind(&data.worktree_path)
        .bind(data.mode)
        .fetch_one(pool)
        .await
    }

    /// Compare-and-swap status transition. Atomically updates the row only if
    /// its current `status` equals `expected_from`. Returns the updated row,
    /// or `CasRejected` when no row matched (either missing, deleted, or in a
    /// different status — caller should re-read to disambiguate).
    ///
    /// Bumps `updated_at`. Does NOT touch `last_used_at` / `merged_at` /
    /// `archived_at`; those are set by dedicated helpers below so audit
    /// timestamps remain precise.
    pub async fn transition_status(
        pool: &SqlitePool,
        id: Uuid,
        expected_from: SessionWorktreeStatus,
        to: SessionWorktreeStatus,
    ) -> Result<Self, SessionWorktreeError> {
        let updated = sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET status = ?2,
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1 AND status = ?3
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(to)
        .bind(expected_from)
        .fetch_optional(pool)
        .await?;

        updated.ok_or(SessionWorktreeError::CasRejected {
            worktree_id: id,
            expected: expected_from,
            actual: None,
        })
    }

    /// Compare-and-swap status transition that also clears transient merge /
    /// cleanup metadata. Used when leaving `merging` /
    /// `needs_conflict_resolution` / `cleanup_failed` to ensure the next
    /// operation starts from a clean slate.
    pub async fn transition_status_clearing_transient(
        pool: &SqlitePool,
        id: Uuid,
        expected_from: SessionWorktreeStatus,
        to: SessionWorktreeStatus,
    ) -> Result<Self, SessionWorktreeError> {
        let updated = sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET status = ?2,
                merge_operation = NULL,
                merge_target_branch = NULL,
                conflict_files_json = '[]',
                operation_started_at = NULL,
                cleanup_error = NULL,
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1 AND status = ?3
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(to)
        .bind(expected_from)
        .fetch_optional(pool)
        .await?;

        updated.ok_or(SessionWorktreeError::CasRejected {
            worktree_id: id,
            expected: expected_from,
            actual: None,
        })
    }

    /// Record merge intent (operation + target branch + start timestamp)
    /// without changing status. Caller should `transition_status` into
    /// `merging` first to keep the audit trail coherent.
    pub async fn set_merge_metadata(
        pool: &SqlitePool,
        id: Uuid,
        operation: SessionWorktreeMergeOperation,
        target_branch: Option<&str>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET merge_operation = ?2,
                merge_target_branch = ?3,
                operation_started_at = datetime('now', 'subsec'),
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(operation)
        .bind(target_branch)
        .fetch_one(pool)
        .await
    }

    /// Replace the conflict-file list. Caller must already be in (or about to
    /// enter) `needs_conflict_resolution`; this helper deliberately does not
    /// guard status so it can be used both entering and updating the state.
    pub async fn set_conflict_files(
        pool: &SqlitePool,
        id: Uuid,
        files: &[String],
    ) -> Result<Self, sqlx::Error> {
        let mut sorted: Vec<String> = files
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        sorted.sort();
        sorted.dedup();
        let payload = serde_json::to_string(&sorted).unwrap_or_else(|_| "[]".to_string());

        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET conflict_files_json = ?2,
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(payload)
        .fetch_one(pool)
        .await
    }

    /// Record the integration cursor. It starts as the HEAD of `base_branch`
    /// when the worktree is created and advances to the latest integrated
    /// session commit after a cherry-pick merge.
    pub async fn record_base_commit(
        pool: &SqlitePool,
        id: Uuid,
        commit: Option<&str>,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET base_commit = ?2,
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(commit)
        .fetch_one(pool)
        .await
    }

    /// Bump `last_used_at`. Called by the runner whenever an agent run
    /// resolves to this worktree, so the janitor can identify stale rows.
    pub async fn touch_last_used(pool: &SqlitePool, id: Uuid) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET last_used_at = datetime('now', 'subsec'),
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .fetch_one(pool)
        .await
    }

    /// Stamp `merged_at` and clear resolved conflict metadata when
    /// transitioning into `merged`.
    pub async fn record_merged_at(pool: &SqlitePool, id: Uuid) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET merged_at = datetime('now', 'subsec'),
                conflict_files_json = '[]',
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .fetch_one(pool)
        .await
    }

    /// Stamp `archived_at` when transitioning into `archived`.
    pub async fn record_archived_at(pool: &SqlitePool, id: Uuid) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET archived_at = datetime('now', 'subsec'),
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .fetch_one(pool)
        .await
    }

    /// Record the error produced by `git worktree remove` when cleanup fails.
    /// Caller should already have transitioned into `cleanup_failed`.
    pub async fn set_cleanup_error(
        pool: &SqlitePool,
        id: Uuid,
        error: &str,
    ) -> Result<Self, sqlx::Error> {
        sqlx::query_as::<_, Self>(&format!(
            r#"
            UPDATE chat_session_worktrees
            SET cleanup_error = ?2,
                updated_at = datetime('now', 'subsec')
            WHERE id = ?1
            {CHAT_SESSION_WORKTREE_RETURNING}
            "#
        ))
        .bind(id)
        .bind(error)
        .fetch_one(pool)
        .await
    }
}

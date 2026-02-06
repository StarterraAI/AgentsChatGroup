use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatArtifact {
    pub id: Uuid,
    pub session_id: Uuid,
    pub name: String,
    pub path: String,
    pub r#type: String,
    pub created_by: Option<Uuid>,
    pub pinned: bool,
    pub created_at: DateTime<Utc>,
}

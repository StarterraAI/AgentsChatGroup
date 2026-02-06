use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Type};
use ts_rs::TS;
use uuid::Uuid;

#[derive(Debug, Clone, Type, Serialize, Deserialize, PartialEq, TS)]
#[sqlx(type_name = "chat_permission_ttl_type", rename_all = "lowercase")]
#[serde(rename_all = "lowercase")]
#[ts(use_ts_enum)]
pub enum ChatPermissionTtlType {
    Once,
    Time,
    Session,
}

#[derive(Debug, Clone, FromRow, Serialize, Deserialize, TS)]
pub struct ChatPermission {
    pub id: Uuid,
    pub session_id: Uuid,
    pub session_agent_id: Uuid,
    pub capability: String,
    #[ts(type = "JsonValue")]
    pub scope: sqlx::types::Json<serde_json::Value>,
    pub ttl_type: ChatPermissionTtlType,
    pub expires_at: Option<DateTime<Utc>>,
    pub granted_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

use anyhow::Result;
use chrono::DateTime;
use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadPlanItemStatus {
    Pending,
    InProgress,
    Completed,
}

impl ThreadPlanItemStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    pub fn parse(value: &str) -> Result<Self> {
        match value {
            "pending" => Ok(Self::Pending),
            "in_progress" => Ok(Self::InProgress),
            "completed" => Ok(Self::Completed),
            _ => Err(anyhow::anyhow!("invalid thread plan item status: {value}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlanSnapshot {
    pub id: String,
    pub thread_id: String,
    pub source_turn_id: String,
    pub source_item_id: String,
    pub raw_markdown: String,
    pub created_at: DateTime<Utc>,
    pub superseded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlanItem {
    pub snapshot_id: String,
    pub row_id: String,
    pub row_index: i64,
    pub status: ThreadPlanItemStatus,
    pub step: String,
    pub path: String,
    pub details: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub depends_on: Vec<String>,
    pub acceptance: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveThreadPlan {
    pub snapshot: ThreadPlanSnapshot,
    pub items: Vec<ThreadPlanItem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlanSnapshotCreateParams {
    pub id: String,
    pub thread_id: String,
    pub source_turn_id: String,
    pub source_item_id: String,
    pub raw_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadPlanItemCreateParams {
    pub row_id: String,
    pub row_index: i64,
    pub status: ThreadPlanItemStatus,
    pub step: String,
    pub path: String,
    pub details: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    pub depends_on: Vec<String>,
    pub acceptance: Option<String>,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct ThreadPlanSnapshotRow {
    pub(crate) id: String,
    pub(crate) thread_id: String,
    pub(crate) source_turn_id: String,
    pub(crate) source_item_id: String,
    pub(crate) raw_markdown: String,
    pub(crate) created_at: i64,
    pub(crate) superseded_at: Option<i64>,
}

#[derive(Debug, sqlx::FromRow)]
pub(crate) struct ThreadPlanItemRow {
    pub(crate) snapshot_id: String,
    pub(crate) row_id: String,
    pub(crate) row_index: i64,
    pub(crate) status: String,
    pub(crate) step: String,
    pub(crate) path: String,
    pub(crate) details: String,
    pub(crate) inputs: String,
    pub(crate) outputs: String,
    pub(crate) depends_on: String,
    pub(crate) acceptance: Option<String>,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
    pub(crate) completed_at: Option<i64>,
}

impl TryFrom<ThreadPlanSnapshotRow> for ThreadPlanSnapshot {
    type Error = anyhow::Error;

    fn try_from(value: ThreadPlanSnapshotRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: value.id,
            thread_id: value.thread_id,
            source_turn_id: value.source_turn_id,
            source_item_id: value.source_item_id,
            raw_markdown: value.raw_markdown,
            created_at: epoch_seconds_to_datetime(value.created_at)?,
            superseded_at: value
                .superseded_at
                .map(epoch_seconds_to_datetime)
                .transpose()?,
        })
    }
}

impl TryFrom<ThreadPlanItemRow> for ThreadPlanItem {
    type Error = anyhow::Error;

    fn try_from(value: ThreadPlanItemRow) -> Result<Self, Self::Error> {
        Ok(Self {
            snapshot_id: value.snapshot_id,
            row_id: value.row_id,
            row_index: value.row_index,
            status: ThreadPlanItemStatus::parse(value.status.as_str())?,
            step: value.step,
            path: value.path,
            details: value.details,
            inputs: parse_string_list(value.inputs.as_str())?,
            outputs: parse_string_list(value.outputs.as_str())?,
            depends_on: parse_string_list(value.depends_on.as_str())?,
            acceptance: value.acceptance,
            created_at: epoch_seconds_to_datetime(value.created_at)?,
            updated_at: epoch_seconds_to_datetime(value.updated_at)?,
            completed_at: value
                .completed_at
                .map(epoch_seconds_to_datetime)
                .transpose()?,
        })
    }
}

fn epoch_seconds_to_datetime(secs: i64) -> Result<DateTime<Utc>> {
    DateTime::<Utc>::from_timestamp(secs, 0)
        .ok_or_else(|| anyhow::anyhow!("invalid unix timestamp: {secs}"))
}

fn parse_string_list(raw: &str) -> Result<Vec<String>> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    serde_json::from_str(raw).map_err(|err| anyhow::anyhow!("invalid thread plan list json: {err}"))
}

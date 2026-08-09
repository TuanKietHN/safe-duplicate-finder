//! Sealed keeper plans and reviewable dry-run projections.

use std::path::PathBuf;

use chrono::Utc;
use dedupe_core::{
    DedupeError, Result,
    model::{DuplicateGroup, KeepPolicy, MemberAction, ProvenFile},
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::{Database, DuplicateRepository, duplicates::snapshot_id_for_path};

/// Immutable dry-run aggregate for a sealed plan.
#[derive(Debug, Clone, Serialize)]
pub struct PlanSummary {
    /// Plan identifier.
    pub plan_id: Uuid,
    /// Owning scan session.
    pub session_id: Uuid,
    /// Current plan state.
    pub status: String,
    /// Number of groups represented.
    pub groups: u64,
    /// Number of files proposed for quarantine.
    pub quarantine_files: u64,
    /// Exact total bytes proposed for quarantine.
    pub quarantine_bytes: u64,
}

/// Most recent sealed workflow identifiers used to restore desktop navigation after restart.
#[derive(Debug, Clone, Serialize)]
pub struct LatestPlanContext {
    /// Project owning the scan session.
    pub project_id: Uuid,
    /// Completed scan session used to build the plan.
    pub session_id: Uuid,
    /// Newest sealed plan for that session.
    pub plan_id: Uuid,
}

/// One immutable quarantine item loaded with its proven evidence.
#[derive(Debug, Clone)]
pub struct PlannedQuarantineItem {
    /// Plan-item identifier used for idempotency.
    pub plan_item_id: Uuid,
    /// Owning duplicate group.
    pub group_id: Uuid,
    /// Proven file authorized by the plan.
    pub file: ProvenFile,
}

/// Operation-plan repository.
#[derive(Debug, Clone)]
pub struct PlanRepository {
    database: Database,
}

impl PlanRepository {
    /// Bind the repository to a database.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// Persist all decisions and seal only after every group has a keeper.
    pub fn create_and_seal(
        &self,
        session_id: Uuid,
        policy: &KeepPolicy,
        groups: &[DuplicateGroup],
    ) -> Result<Uuid> {
        for group in groups {
            group.validate_keeper()?;
            if group
                .members
                .iter()
                .any(|member| member.action == MemberAction::Manual)
            {
                return Err(DedupeError::Safety(format!(
                    "Nhóm {} vẫn còn thành viên chưa được quyết định",
                    group.id
                )));
            }
        }
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        transaction
            .execute(
                "INSERT INTO operation_plans (
                    id,session_id,status,policy,evidence_version,created_at
                 ) VALUES (?1,?2,'draft',?3,1,?4)",
                params![
                    id.to_string(),
                    session_id.to_string(),
                    policy_name(policy),
                    now,
                ],
            )
            .map_err(store_error)?;
        for group in groups {
            for member in &group.members {
                let snapshot_id =
                    snapshot_id_for_path(&transaction, session_id, &member.file.metadata.path)?;
                transaction
                    .execute(
                        "INSERT INTO plan_items (
                            id,plan_id,group_id,snapshot_id,action,reason,selected_by
                         ) VALUES (?1,?2,?3,?4,?5,?6,'policy')",
                        params![
                            Uuid::new_v4().to_string(),
                            id.to_string(),
                            group.id.to_string(),
                            snapshot_id,
                            action_name(member.action)?,
                            member.reason,
                        ],
                    )
                    .map_err(store_error)?;
            }
        }
        transaction
            .execute(
                "UPDATE operation_plans SET status='sealed',sealed_at=?1 WHERE id=?2",
                params![now, id.to_string()],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(id)
    }

    /// Calculate a dry-run summary without touching the filesystem.
    pub fn summary(&self, plan_id: Uuid) -> Result<PlanSummary> {
        let connection = self.database.connection();
        let (session, status): (String, String) = connection
            .query_row(
                "SELECT session_id,status FROM operation_plans WHERE id=?1",
                [plan_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(store_error)?
            .ok_or_else(|| {
                DedupeError::InvalidInput(format!("Không nhận diện được kế hoạch {plan_id}"))
            })?;
        let (groups, files, bytes): (i64, i64, i64) = connection
            .query_row(
                "SELECT COUNT(DISTINCT p.group_id),COUNT(*),COALESCE(SUM(s.size_bytes),0)
                 FROM plan_items p JOIN file_snapshots s ON s.id=p.snapshot_id
                 WHERE p.plan_id=?1 AND p.action='quarantine'",
                [plan_id.to_string()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(store_error)?;
        Ok(PlanSummary {
            plan_id,
            session_id: parse_uuid(&session, "session")?,
            status,
            groups: to_u64(groups, "số nhóm")?,
            quarantine_files: to_u64(files, "số tệp cách ly")?,
            quarantine_bytes: to_u64(bytes, "số byte cách ly")?,
        })
    }

    /// Return the newest sealed plan for one completed scan session, if any.
    pub fn latest_sealed_for_session(&self, session_id: Uuid) -> Result<Option<Uuid>> {
        let value: Option<String> = self
            .database
            .connection()
            .query_row(
                "SELECT id FROM operation_plans
                 WHERE session_id=?1 AND status='sealed'
                 ORDER BY sealed_at DESC,id DESC LIMIT 1",
                [session_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        value.map(|id| parse_uuid(&id, "plan")).transpose()
    }

    /// Return the newest sealed plan together with its owning project and session.
    pub fn latest_sealed_context(&self) -> Result<Option<LatestPlanContext>> {
        let value: Option<(String, String, String)> = self
            .database
            .connection()
            .query_row(
                "SELECT s.project_id,p.session_id,p.id
                 FROM operation_plans p
                 JOIN scan_sessions s ON s.id=p.session_id
                 WHERE p.status='sealed'
                 ORDER BY p.sealed_at DESC,p.id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(store_error)?;
        value
            .map(|(project, session, plan)| {
                Ok(LatestPlanContext {
                    project_id: parse_uuid(&project, "project")?,
                    session_id: parse_uuid(&session, "session")?,
                    plan_id: parse_uuid(&plan, "plan")?,
                })
            })
            .transpose()
    }

    /// Load the plan's quarantine members and their immutable digest evidence.
    pub fn quarantine_items(&self, plan_id: Uuid) -> Result<Vec<PlannedQuarantineItem>> {
        let summary = self.summary(plan_id)?;
        if summary.status != "sealed" && summary.status != "executing" {
            return Err(DedupeError::Safety(format!(
                "Kế hoạch {plan_id} chưa được khóa"
            )));
        }
        let groups = DuplicateRepository::new(self.database.clone())
            .load_session_groups(summary.session_id)?;
        let connection = self
            .database
            .read_connection()
            .map_err(|error| DedupeError::Durability(error.to_string()))?;
        let mut statement = connection
            .prepare(
                "SELECT p.id,p.group_id,e.original_path
                 FROM plan_items p
                 JOIN file_snapshots s ON s.id=p.snapshot_id
                 JOIN file_entries e ON e.id=s.file_entry_id
                 WHERE p.plan_id=?1 AND p.action='quarantine'
                 ORDER BY p.group_id,e.normalized_path",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([plan_id.to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(store_error)?;
        let mut selected = Vec::new();
        for row in rows {
            let (item_id, group_id, path) = row.map_err(store_error)?;
            selected.push((
                parse_uuid(&item_id, "plan item")?,
                parse_uuid(&group_id, "group")?,
                PathBuf::from(path),
            ));
        }
        selected
            .into_iter()
            .map(|(plan_item_id, group_id, path)| {
                let file = groups
                    .iter()
                    .find(|group| group.id == group_id)
                    .and_then(|group| {
                        group
                            .members
                            .iter()
                            .find(|member| member.file.metadata.path == path)
                    })
                    .map(|member| member.file.clone())
                    .ok_or_else(|| {
                        DedupeError::State(format!(
                            "Mục kế hoạch {plan_item_id} không có thành viên đã chứng minh tương ứng"
                        ))
                    })?;
                Ok(PlannedQuarantineItem {
                    plan_item_id,
                    group_id,
                    file,
                })
            })
            .collect()
    }

    /// Resolve the project owning a plan.
    pub fn project_id(&self, plan_id: Uuid) -> Result<Uuid> {
        let value: String = self
            .database
            .connection()
            .query_row(
                "SELECT s.project_id FROM operation_plans p
                 JOIN scan_sessions s ON s.id=p.session_id WHERE p.id=?1",
                [plan_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?
            .ok_or_else(|| {
                DedupeError::InvalidInput(format!("Không nhận diện được kế hoạch {plan_id}"))
            })?;
        parse_uuid(&value, "project")
    }

    /// Mark a sealed plan as executing before its first mutation.
    pub fn mark_executing(&self, plan_id: Uuid) -> Result<()> {
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE operation_plans SET status='executing' WHERE id=?1 AND status='sealed'",
                [plan_id.to_string()],
            )
            .map_err(store_error)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DedupeError::Safety(
                "Kế hoạch chưa được khóa hoặc đã được thực thi".into(),
            ))
        }
    }

    /// Mark a sealed plan stale after a read-only evidence recheck fails.
    pub fn mark_stale(&self, plan_id: Uuid) -> Result<()> {
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE operation_plans SET status='stale' WHERE id=?1 AND status='sealed'",
                [plan_id.to_string()],
            )
            .map_err(store_error)?;
        if changed == 1 {
            Ok(())
        } else {
            Err(DedupeError::Safety(
                "Chỉ kế hoạch đã khóa mới có thể được đánh dấu lỗi thời".into(),
            ))
        }
    }

    /// Mark a plan completed only after all planned transactions are verified.
    pub fn mark_completed_if_verified(&self, plan_id: Uuid) -> Result<bool> {
        let pending: i64 = self
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM plan_items p
                 LEFT JOIN file_transactions t
                   ON t.plan_item_id=p.id AND t.kind='quarantine' AND t.status='verified'
                 WHERE p.plan_id=?1 AND p.action='quarantine' AND t.id IS NULL",
                [plan_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if pending != 0 {
            return Ok(false);
        }
        self.database
            .connection()
            .execute(
                "UPDATE operation_plans SET status='completed',completed_at=?1
                 WHERE id=?2 AND status='executing'",
                params![Utc::now().to_rfc3339(), plan_id.to_string()],
            )
            .map_err(store_error)?;
        Ok(true)
    }
}

fn policy_name(policy: &KeepPolicy) -> &'static str {
    match policy {
        KeepPolicy::Default { .. } => "default",
        KeepPolicy::Oldest => "oldest",
        KeepPolicy::Newest => "newest",
        KeepPolicy::ShortestPath => "shortest",
        KeepPolicy::Manual(_) => "manual",
    }
}

fn action_name(action: MemberAction) -> Result<&'static str> {
    match action {
        MemberAction::Keep => Ok("keep"),
        MemberAction::Quarantine => Ok("quarantine"),
        MemberAction::Manual => Err(DedupeError::Safety(
            "Thành viên xem xét thủ công không thể vào kế hoạch đã khóa".into(),
        )),
    }
}

fn parse_uuid(value: &str, kind: &str) -> Result<Uuid> {
    let label = match kind {
        "session" => "phiên quét",
        "plan item" => "mục kế hoạch",
        "group" => "nhóm",
        "project" => "dự án",
        "plan" => "kế hoạch",
        _ => kind,
    };
    Uuid::parse_str(value)
        .map_err(|error| DedupeError::State(format!("UUID {label} đã lưu không hợp lệ: {error}")))
}

fn to_u64(value: i64, field: &str) -> Result<u64> {
    u64::try_from(value).map_err(|_| DedupeError::State(format!("Giá trị {field} đã lưu là số âm")))
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}

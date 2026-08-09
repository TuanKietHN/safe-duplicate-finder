//! Project and source-root persistence.

use std::path::{Path, PathBuf};

use chrono::Utc;
use dedupe_core::{
    DedupeError, Result,
    filters::{CompiledFilter, FilterConfig},
    model::{ComparisonMode, WorkerConfig},
    path_normalization::{normalize_path, path_key},
};
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use uuid::Uuid;

use crate::Database;

/// Project repository.
#[derive(Debug, Clone)]
pub struct ProjectRepository {
    database: Database,
}

/// Project row used by CLI and desktop adapters.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectRecord {
    /// Project identifier.
    pub id: Uuid,
    /// User-visible name.
    pub name: String,
    /// Active comparison mode.
    pub mode: ComparisonMode,
    /// Global bounded worker limit used by enumeration and hash scheduling.
    pub worker_limit: usize,
    /// Active or archived.
    pub status: String,
    /// Most recent completed scan timestamp.
    pub last_scan_at: Option<String>,
}

/// Enabled source-root row for project configuration screens.
#[derive(Debug, Clone, Serialize)]
pub struct ProjectRootRecord {
    /// Root identifier.
    pub id: Uuid,
    /// Absolute source path.
    pub path: PathBuf,
    /// Whether default keeper policy prefers this root.
    pub primary: bool,
}

impl ProjectRepository {
    /// Bind repository to a database.
    #[must_use]
    pub fn new(database: Database) -> Self {
        Self { database }
    }

    /// Create a project with conservative defaults.
    pub fn create(&self, name: &str, mode: ComparisonMode) -> Result<Uuid> {
        if name.trim().is_empty() {
            return Err(DedupeError::InvalidInput(
                "Tên dự án không được để trống".into(),
            ));
        }
        let id = Uuid::new_v4();
        let now = Utc::now().to_rfc3339();
        self.database
            .connection()
            .execute(
                "INSERT INTO projects (id,name,mode,created_at,updated_at) VALUES (?1,?2,?3,?4,?4)",
                params![id.to_string(), name.trim(), mode_name(mode), now],
            )
            .map_err(store_error)?;
        Ok(id)
    }

    /// Rename or change the mode of an active project without starting a scan.
    pub fn update(&self, project_id: Uuid, name: &str, mode: ComparisonMode) -> Result<()> {
        if name.trim().is_empty() {
            return Err(DedupeError::InvalidInput(
                "Tên dự án không được để trống".into(),
            ));
        }
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE projects SET name=?1,mode=?2,updated_at=?3
                 WHERE id=?4 AND status='active'",
                params![
                    name.trim(),
                    mode_name(mode),
                    Utc::now().to_rfc3339(),
                    project_id.to_string()
                ],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::InvalidInput(format!(
                "Không tìm thấy dự án đang hoạt động: {project_id}"
            )));
        }
        Ok(())
    }

    /// Set the persisted global worker limit without starting a scan.
    pub fn set_worker_limit(&self, project_id: Uuid, worker_limit: usize) -> Result<()> {
        if !(1..=64).contains(&worker_limit) {
            return Err(DedupeError::InvalidInput(
                "Giới hạn luồng xử lý phải từ 1 đến 64".into(),
            ));
        }
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE projects SET worker_limit=?1,updated_at=?2
                 WHERE id=?3 AND status='active'",
                params![
                    i64::try_from(worker_limit).map_err(|error| {
                        DedupeError::InvalidInput(format!(
                            "Giới hạn luồng xử lý không hợp lệ: {error}"
                        ))
                    })?,
                    Utc::now().to_rfc3339(),
                    project_id.to_string()
                ],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::InvalidInput(format!(
                "Không tìm thấy dự án đang hoạt động: {project_id}"
            )));
        }
        Ok(())
    }

    /// Load scheduler settings from the persisted global limit.
    ///
    /// Until a volume has an explicit storage profile, full reads use one worker per volume. This is
    /// the safe HDD/network/unknown default; metadata work may use the complete configured pool.
    pub fn worker_config(&self, project_id: Uuid) -> Result<WorkerConfig> {
        let worker_limit: i64 = self
            .database
            .connection()
            .query_row(
                "SELECT worker_limit FROM projects WHERE id=?1 AND status='active'",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        let metadata_workers = usize::try_from(worker_limit).map_err(|error| {
            DedupeError::State(format!("Giới hạn luồng đã lưu không hợp lệ: {error}"))
        })?;
        if !(1..=64).contains(&metadata_workers) {
            return Err(DedupeError::State(format!(
                "Giới hạn luồng đã lưu nằm ngoài khoảng 1..=64: {metadata_workers}"
            )));
        }
        Ok(WorkerConfig {
            metadata_workers,
            full_hash_workers_per_volume: 1,
            queue_capacity: 1024,
        })
    }

    /// Archive only the project record; source files and quarantine inventory are untouched.
    pub fn archive(&self, project_id: Uuid) -> Result<()> {
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE projects SET status='archived',updated_at=?1
                 WHERE id=?2 AND status='active'",
                params![Utc::now().to_rfc3339(), project_id.to_string()],
            )
            .map_err(store_error)?;
        if changed != 1 {
            let status = self
                .database
                .connection()
                .query_row(
                    "SELECT status FROM projects WHERE id=?1",
                    [project_id.to_string()],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(store_error)?;
            if status.as_deref() == Some("archived") {
                return Ok(());
            }
            return Err(DedupeError::InvalidInput(format!(
                "Không tìm thấy dự án đang hoạt động: {project_id}"
            )));
        }
        Ok(())
    }

    /// Atomically replace an active project's scan filters without starting a scan.
    pub fn replace_filter_config(&self, project_id: Uuid, config: &FilterConfig) -> Result<()> {
        let config = canonical_filter_config(config.clone());
        let _validated = CompiledFilter::new(config.clone())?;
        let mut connection = self.database.connection();
        let transaction = connection.transaction().map_err(store_error)?;
        let active: i64 = transaction
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id=?1 AND status='active'",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if active != 1 {
            return Err(DedupeError::InvalidInput(format!(
                "Không tìm thấy dự án đang hoạt động: {project_id}"
            )));
        }
        transaction
            .execute(
                "DELETE FROM filter_rules WHERE project_id=?1",
                [project_id.to_string()],
            )
            .map_err(store_error)?;
        let now = Utc::now().to_rfc3339();
        let insert = |kind: &str, value: &str| -> Result<()> {
            transaction
                .execute(
                    "INSERT INTO filter_rules (id,project_id,kind,value,created_at)
                     VALUES (?1,?2,?3,?4,?5)",
                    params![
                        Uuid::new_v4().to_string(),
                        project_id.to_string(),
                        kind,
                        value,
                        now
                    ],
                )
                .map_err(store_error)?;
            Ok(())
        };
        if config.include_extensions.is_empty() {
            insert("include_extension", "*")?;
        } else {
            for extension in &config.include_extensions {
                insert("include_extension", extension)?;
            }
        }
        for extension in &config.exclude_extensions {
            insert("exclude_extension", extension)?;
        }
        for pattern in &config.exclude_globs {
            insert("exclude_glob", pattern)?;
        }
        insert("minimum_size", &config.minimum_size.to_string())?;
        insert("skip_hidden", bool_text(config.skip_hidden))?;
        insert("skip_system", bool_text(config.skip_system))?;
        insert("skip_quarantine", "true")?;
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Load a project's persistent filter configuration, or safety-first defaults for a new project.
    pub fn filter_config(&self, project_id: Uuid) -> Result<FilterConfig> {
        let connection = self.database.connection();
        let mut statement = connection
            .prepare(
                "SELECT kind,value FROM filter_rules
                 WHERE project_id=?1 AND enabled=1 ORDER BY created_at,id",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([project_id.to_string()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(store_error)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(store_error)?;
        if rows.is_empty() {
            return Ok(FilterConfig::default());
        }
        let mut config = FilterConfig {
            include_extensions: Vec::new(),
            exclude_extensions: Vec::new(),
            exclude_globs: Vec::new(),
            minimum_size: 0,
            skip_hidden: true,
            skip_system: true,
        };
        for (kind, value) in rows {
            match kind.as_str() {
                "include_extension" if value != "*" => config.include_extensions.push(value),
                "exclude_extension" => config.exclude_extensions.push(value),
                "exclude_glob" | "exclude_folder" => config.exclude_globs.push(value),
                "minimum_size" => {
                    config.minimum_size = value.parse().map_err(|error| {
                        DedupeError::State(format!(
                            "Kích thước tối thiểu đã lưu không hợp lệ: {error}"
                        ))
                    })?;
                }
                "skip_hidden" => config.skip_hidden = parse_bool(&value)?,
                "skip_system" => config.skip_system = parse_bool(&value)?,
                "skip_quarantine" => {}
                unknown => {
                    return Err(DedupeError::State(format!(
                        "Không nhận diện được loại bộ lọc đã lưu: {unknown}"
                    )));
                }
            }
        }
        let config = canonical_filter_config(config);
        let _validated = CompiledFilter::new(config.clone())?;
        Ok(config)
    }

    /// Add a root without scanning. The database itself and quarantine roots are rejected.
    pub fn add_root(&self, project_id: Uuid, path: &Path, primary: bool) -> Result<Uuid> {
        let active: i64 = self
            .database
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM projects WHERE id=?1 AND status='active'",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        if active != 1 {
            return Err(DedupeError::InvalidInput(format!(
                "Không tìm thấy dự án đang hoạt động: {project_id}"
            )));
        }
        let absolute = std::path::absolute(path)
            .map_err(|error| DedupeError::io("xác định thư mục nguồn", path, error))?;
        if is_link_or_reparse_root(&absolute)? {
            return Err(DedupeError::Safety(format!(
                "Thư mục gốc là symlink hoặc junction bị tắt theo mặc định: {}",
                absolute.display()
            )));
        }
        if !absolute.is_dir() {
            return Err(DedupeError::InvalidInput(format!(
                "Thư mục nguồn không truy cập được: {}",
                absolute.display()
            )));
        }
        if self.database.path().starts_with(&absolute) || absolute.starts_with(self.database.path())
        {
            return Err(DedupeError::Safety(
                "Cơ sở dữ liệu và thư mục nguồn không được chứa lẫn nhau".into(),
            ));
        }
        if absolute.components().any(|component| {
            component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(".safe-duplicate-finder-quarantine")
        }) {
            return Err(DedupeError::Safety(
                "Không thể thêm thư mục cách ly làm thư mục nguồn".into(),
            ));
        }
        let normalized = normalize_path(&absolute)?;
        {
            let connection = self.database.connection();
            let mut statement = connection
                .prepare(
                    "SELECT original_path,normalized_path FROM project_roots
                     WHERE project_id=?1 AND enabled=1",
                )
                .map_err(store_error)?;
            let rows = statement
                .query_map([project_id.to_string()], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(store_error)?;
            for row in rows {
                let (existing_path, existing) = row.map_err(store_error)?;
                if dedupe_core::path_normalization::is_same_or_child(&existing, &normalized)
                    || dedupe_core::path_normalization::is_same_or_child(&normalized, &existing)
                {
                    return Err(DedupeError::Safety(format!(
                        "Thư mục nguồn chồng lấn với thư mục đã cấu hình {existing_path}"
                    )));
                }
            }
        }
        let id = Uuid::new_v4();
        self.database
            .connection()
            .execute(
                "INSERT INTO project_roots (
                    id, project_id, original_path, normalized_path, path_key,
                    is_primary, validation_status, created_at
                ) VALUES (?1,?2,?3,?4,?5,?6,'pending',?7)",
                params![
                    id.to_string(),
                    project_id.to_string(),
                    absolute.to_string_lossy(),
                    normalized,
                    path_key(&absolute)?.as_slice(),
                    i64::from(primary),
                    Utc::now().to_rfc3339(),
                ],
            )
            .map_err(store_error)?;
        Ok(id)
    }

    /// List enabled roots for a project.
    pub fn roots(&self, project_id: Uuid) -> Result<Vec<(Uuid, PathBuf, bool)>> {
        let connection = self.database.connection();
        let mut statement = connection
            .prepare(
                "SELECT id, original_path, is_primary FROM project_roots
                 WHERE project_id=?1 AND enabled=1 ORDER BY is_primary DESC, original_path",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([project_id.to_string()], |row| {
                let id: String = row.get(0)?;
                let path: String = row.get(1)?;
                let primary: i64 = row.get(2)?;
                Ok((id, PathBuf::from(path), primary != 0))
            })
            .map_err(store_error)?;
        rows.map(|row| {
            let (id, path, primary) = row.map_err(store_error)?;
            let id = Uuid::parse_str(&id).map_err(|error| {
                DedupeError::State(format!("UUID thư mục gốc không hợp lệ: {error}"))
            })?;
            Ok((id, path, primary))
        })
        .collect()
    }

    /// List enabled roots in a frontend-friendly structure.
    pub fn root_records(&self, project_id: Uuid) -> Result<Vec<ProjectRootRecord>> {
        Ok(self
            .roots(project_id)?
            .into_iter()
            .map(|(id, path, primary)| ProjectRootRecord { id, path, primary })
            .collect())
    }

    /// Disable one configured root. No filesystem operation is performed.
    pub fn remove_root(&self, project_id: Uuid, root_id: Uuid) -> Result<()> {
        let changed = self
            .database
            .connection()
            .execute(
                "UPDATE project_roots SET enabled=0
                 WHERE id=?1 AND project_id=?2 AND enabled=1",
                params![root_id.to_string(), project_id.to_string()],
            )
            .map_err(store_error)?;
        if changed != 1 {
            return Err(DedupeError::InvalidInput(format!(
                "Không tìm thấy thư mục gốc đang bật: {root_id}"
            )));
        }
        Ok(())
    }

    /// Load one project's configured comparison mode.
    pub fn mode(&self, project_id: Uuid) -> Result<ComparisonMode> {
        let value: String = self
            .database
            .connection()
            .query_row(
                "SELECT mode FROM projects WHERE id=?1 AND status='active'",
                [project_id.to_string()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        parse_mode(&value)
    }

    /// List projects without starting a scan.
    pub fn list(&self) -> Result<Vec<ProjectRecord>> {
        let connection = self.database.connection();
        let mut statement = connection
            .prepare(
                "SELECT id,name,mode,worker_limit,status,last_scan_at
                 FROM projects ORDER BY created_at,id",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })
            .map_err(store_error)?;
        rows.map(|row| {
            let (id, name, mode, worker_limit, status, last_scan_at) = row.map_err(store_error)?;
            Ok(ProjectRecord {
                id: Uuid::parse_str(&id).map_err(|error| {
                    DedupeError::State(format!("UUID dự án không hợp lệ: {error}"))
                })?,
                name,
                mode: parse_mode(&mode)?,
                worker_limit: usize::try_from(worker_limit).map_err(|error| {
                    DedupeError::State(format!("Giới hạn luồng đã lưu không hợp lệ: {error}"))
                })?,
                status,
                last_scan_at,
            })
        })
        .collect()
    }
}

fn mode_name(mode: ComparisonMode) -> &'static str {
    match mode {
        ComparisonMode::Strict => "strict",
        ComparisonMode::Content => "content",
    }
}

fn parse_mode(value: &str) -> Result<ComparisonMode> {
    match value {
        "strict" => Ok(ComparisonMode::Strict),
        "content" => Ok(ComparisonMode::Content),
        _ => Err(DedupeError::State(format!(
            "Không nhận diện được chế độ dự án đã lưu: {value}"
        ))),
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn parse_bool(value: &str) -> Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(DedupeError::State(format!(
            "Giá trị boolean đã lưu không hợp lệ: {value}"
        ))),
    }
}

fn canonical_filter_config(mut config: FilterConfig) -> FilterConfig {
    config.include_extensions = canonical_extensions(config.include_extensions);
    config.exclude_extensions = canonical_extensions(config.exclude_extensions);
    config.exclude_globs.sort_unstable();
    config.exclude_globs.dedup();
    config
}

fn canonical_extensions(values: Vec<String>) -> Vec<String> {
    let mut values = values
        .into_iter()
        .map(|value| value.trim().trim_start_matches('.').to_lowercase())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    values.sort_unstable();
    values.dedup();
    values
}

fn is_link_or_reparse_root(path: &Path) -> Result<bool> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        DedupeError::io("kiểm tra loại liên kết của thư mục nguồn", path, error)
    })?;
    if metadata.file_type().is_symlink() {
        return Ok(true);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        Ok(metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
    }
    #[cfg(not(windows))]
    Ok(false)
}

fn store_error(error: rusqlite::Error) -> DedupeError {
    DedupeError::Durability(error.to_string())
}

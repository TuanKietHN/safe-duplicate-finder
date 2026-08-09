//! Bộ điều hợp dòng lệnh cho bộ máy an toàn dùng chung.

use std::{fs::File, path::PathBuf, process::ExitCode, sync::Arc};

use anyhow::Context;
use clap::{Parser, Subcommand, ValueEnum};
use dedupe_core::{
    DedupeError,
    control::ControlToken,
    duplicate_detector::confirm_preliminary_group_detailed_with_config,
    filters::CompiledFilter,
    keep_policy,
    model::{ComparisonMode, DuplicateGroup, KeepPolicy},
    permanent_delete,
    ports::ScanSink,
    progress::ProgressCounters,
    quarantine,
    scanner::scan_roots_with_config,
};
use dedupe_platform::PlatformFileSystem;
use dedupe_store::{
    Database, DuplicateRepository, PermanentDeleteRepository, PlanRepository, ProjectRepository,
    ScanControlMonitor, ScanControlRequest, ScanRepository, SqlitePermanentDeleteJournal,
    SqliteScanSink, SqliteTransactionJournal, TransactionRepository,
};
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(
    name = "safe-dedupe",
    version,
    about = "Trình quản lý tệp trùng lặp ưu tiên an toàn",
    long_about = "Trình tìm tệp trùng lặp ưu tiên xử lý cục bộ. Quét luôn chỉ đọc; cách ly và khôi phục là các thao tác rõ ràng, có nhật ký và được xác minh."
)]
struct Cli {
    /// Cơ sở dữ liệu dự án `SQLite`, phải nằm ngoài mọi thư mục nguồn/cách ly.
    #[arg(long, global = true, default_value = "safe-dedupe.db")]
    database: PathBuf,

    /// Thư mục nhật ký cục bộ có cấu trúc và dễ đọc.
    #[arg(long, global = true)]
    log_directory: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Kiểm tra migration cơ sở dữ liệu và các pragma an toàn.
    Check,
    /// Tạo bản sao lưu `SQLite` nhất quán, không ghi đè.
    Backup {
        /// Đường dẫn bản sao lưu mới; không bao giờ thay thế tệp đã tồn tại.
        #[arg(long)]
        destination: PathBuf,
    },
    /// Tạo/liệt kê dự án và cấu hình thư mục gốc.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Bắt đầu quét chỉ đọc.
    Scan {
        #[command(subcommand)]
        command: ScanCommand,
    },
    /// Xem các nhóm trùng lặp đã chứng minh.
    Results {
        #[command(subcommand)]
        command: ResultsCommand,
    },
    /// Tạo và kiểm tra kế hoạch giữ tệp bất biến.
    Plan {
        #[command(subcommand)]
        command: PlanCommand,
    },
    /// Hiển thị chính xác tác động của kế hoạch mà không thay đổi hệ thống tệp.
    DryRun {
        /// Mã kế hoạch đã khóa.
        #[arg(long)]
        plan: Uuid,
        /// Xuất JSON thay cho văn bản dễ đọc.
        #[arg(long)]
        json: bool,
    },
    /// Áp dụng kế hoạch đã khóa hoặc liệt kê dữ liệu cách ly đã xác minh.
    Quarantine {
        #[command(subcommand)]
        command: QuarantineCommand,
    },
    /// Khôi phục một mục, nhóm trùng, phiên quét hoặc dự án mà không ghi đè.
    Restore {
        /// Mã mục cách ly đã xác minh.
        #[arg(long, conflicts_with_all = ["session", "group", "project"], required_unless_present_any = ["session", "group", "project"])]
        entry: Option<Uuid>,
        /// Khôi phục mọi mục đã xác minh của một phiên quét.
        #[arg(long, conflicts_with_all = ["entry", "group", "project"])]
        session: Option<Uuid>,
        /// Khôi phục mọi mục đã xác minh của một nhóm trùng.
        #[arg(long, conflicts_with_all = ["entry", "session", "project"])]
        group: Option<Uuid>,
        /// Khôi phục mọi mục đã xác minh của một dự án.
        #[arg(long, conflicts_with_all = ["entry", "session", "group"])]
        project: Option<Uuid>,
        /// Token xác nhận chính xác.
        #[arg(long)]
        confirm: String,
    },
    /// Kiểm tra hoặc đối soát các giao dịch bị gián đoạn.
    Recover {
        #[command(subcommand)]
        command: RecoverCommand,
    },
    /// Xuất các nhóm đã chứng minh.
    Report {
        #[command(subcommand)]
        command: ReportCommand,
    },
}

#[derive(Debug, Subcommand)]
enum ProjectCommand {
    /// Tạo dự án. So sánh nghiêm ngặt là mặc định.
    Create {
        #[arg(long)]
        name: String,
        #[arg(long, value_enum, default_value_t = ModeArg::Strict)]
        mode: ModeArg,
    },
    /// Thêm thư mục nguồn mà không quét.
    AddRoot {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        path: PathBuf,
        #[arg(long)]
        primary: bool,
    },
    /// Đổi tên dự án đang hoạt động hoặc thay chế độ so sánh.
    Update {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        name: String,
        #[arg(long, value_enum)]
        mode: ModeArg,
    },
    /// Cấu hình nhóm luồng quét toàn cục cố định.
    SetWorkers {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        workers: usize,
    },
    /// Gỡ cấu hình thư mục gốc mà không chạm vào dữ liệu nguồn.
    RemoveRoot {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        root: Uuid,
    },
    /// Lưu trữ hồ sơ dự án mà không chạm vào dữ liệu nguồn hoặc cách ly.
    Archive {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        confirm: String,
    },
    /// Liệt kê dự án.
    List {
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ScanCommand {
    /// Liệt kê và chứng minh tệp trùng. Lệnh này không bao giờ thay đổi tệp nguồn.
    Start {
        #[arg(long)]
        project: Uuid,
        /// Ghi đè chế độ của dự án trong phiên này.
        #[arg(long, value_enum)]
        mode: Option<ModeArg>,
        /// Xác nhận bắt buộc khi chọn chế độ chỉ so sánh nội dung.
        #[arg(long)]
        acknowledge_content_mode: bool,
        /// Quét mọi phần mở rộng thay cho mặc định PDF/EPUB/MOBI.
        #[arg(long)]
        all_files: bool,
        /// Bỏ qua tệp nhỏ hơn kích thước byte chính xác này.
        #[arg(long)]
        minimum_size: Option<u64>,
    },
    /// Yêu cầu phiên quét tạm dừng tại ranh giới phối hợp tiếp theo.
    Pause {
        #[arg(long)]
        session: Uuid,
    },
    /// Tiếp tục phiên quét đã dừng hoặc khởi động lại phiên chỉ đọc bị gián đoạn.
    Resume {
        #[arg(long)]
        session: Uuid,
    },
    /// Yêu cầu hủy an toàn tại ranh giới phối hợp tiếp theo.
    Cancel {
        #[arg(long)]
        session: Uuid,
    },
    /// Truy vấn một phiên quét bền vững.
    Status {
        #[arg(long)]
        session: Uuid,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ResultsCommand {
    /// Liệt kê nhóm đã chứng minh của một phiên quét hoàn tất.
    List {
        #[arg(long)]
        session: Uuid,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum PlanCommand {
    /// Áp dụng chính sách giữ tệp xác định và khóa quyết định.
    Create {
        #[arg(long)]
        session: Uuid,
        #[arg(long, value_enum, default_value_t = PolicyArg::Default)]
        policy: PolicyArg,
    },
    /// Kiểm tra lại bất biến của kế hoạch đã lưu và hiển thị tổng số.
    Validate {
        #[arg(long)]
        plan: Uuid,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum QuarantineCommand {
    /// Thực thi kế hoạch đã khóa bằng thao tác di chuyển cùng ổ đĩa đã xác minh.
    Apply {
        #[arg(long)]
        plan: Uuid,
        /// Token xác nhận chính xác.
        #[arg(long)]
        confirm: String,
        /// Thư mục cách ly tùy chọn trên cùng ổ đĩa. Mặc định mỗi nguồn có vùng cách ly ẩn riêng.
        #[arg(long)]
        quarantine_root: Option<PathBuf>,
    },
    /// Liệt kê mục cách ly đã xác minh/khôi phục/cần phục hồi.
    List {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        json: bool,
    },
    /// Chuẩn bị token ngắn hạn cho các mục hết hạn lưu giữ được chọn riêng lẻ.
    DeletePrepare {
        /// UUID mục cách ly rõ ràng. Lặp tùy chọn này cho từng tệp đã chọn.
        #[arg(long = "entry", required = true)]
        entries: Vec<Uuid>,
        /// Bỏ qua thời hạn lưu giữ cho lô được chọn rõ ràng này.
        #[arg(long)]
        delete_now: bool,
        /// Xuất thử thách dưới dạng JSON.
        #[arg(long)]
        json: bool,
    },
    /// Thực thi hoặc tiếp tục một lô xóa đã chuẩn bị, chỉ trong vùng cách ly.
    DeleteExecute {
        /// UUID lô đã chuẩn bị.
        #[arg(long)]
        batch: Uuid,
        /// Token ngắn hạn do `delete-prepare` trả về.
        #[arg(long)]
        token: String,
        /// Câu xác nhận chính xác theo phiên bản do `delete-prepare` trả về.
        #[arg(long)]
        confirm: String,
        /// Xuất kết quả dưới dạng JSON.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Subcommand)]
enum RecoverCommand {
    /// Liệt kê mọi giao dịch chưa biết trạng thái cuối.
    Inspect {
        #[arg(long)]
        project: Uuid,
        #[arg(long)]
        json: bool,
    },
    /// Đối soát một giao dịch với thực tế tại nguồn/đích.
    Reconcile {
        #[arg(long)]
        transaction: Uuid,
        #[arg(long)]
        confirm: String,
    },
}

#[derive(Debug, Subcommand)]
enum ReportCommand {
    /// Ghi CSV, JSON hoặc HTML độc lập đã escape.
    Export {
        #[arg(long)]
        session: Uuid,
        #[arg(long, value_enum)]
        format: ReportFormat,
        #[arg(long)]
        destination: PathBuf,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Strict,
    Content,
}

impl From<ModeArg> for ComparisonMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Strict => Self::Strict,
            ModeArg::Content => Self::Content,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PolicyArg {
    Default,
    Oldest,
    Newest,
    Shortest,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ReportFormat {
    Csv,
    Json,
    Html,
}

#[derive(Debug, Serialize)]
struct ScanResult {
    session_id: Uuid,
    discovered_files: u64,
    processed_files: u64,
    bytes_read: u64,
    errors: u64,
    skipped: u64,
    unstable: u64,
    duplicate_groups: usize,
    reclaimable_bytes: u64,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:#}");
            ExitCode::from(exit_code(&error))
        }
    }
}

fn run() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let database = Database::open(&cli.database, &[])?;
    let log_directory = cli.log_directory.unwrap_or_else(|| {
        database
            .path()
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("logs")
    });
    let _log_guards = dedupe_core::logging::init(&log_directory)?;
    // Never serialize command arguments here: confirmations and short-lived delete tokens are
    // intentionally sensitive even though document contents are never logged.
    tracing::info!(database = %database.path().display(), "Đã bắt đầu lệnh CLI");
    match cli.command {
        Command::Check => {
            println!(
                "Kiểm tra cơ sở dữ liệu safe-dedupe đạt: {}",
                database.path().display()
            );
        }
        Command::Backup { destination } => {
            let backup = database.backup_to(&destination)?;
            println!("Đã tạo bản sao lưu cơ sở dữ liệu: {}", backup.display());
        }
        Command::Project { command } => run_project(database, command)?,
        Command::Scan { command } => run_scan(database, command)?,
        Command::Results { command } => run_results(database, command)?,
        Command::Plan { command } => run_plan(database, command)?,
        Command::DryRun { plan, json } => print_plan_summary(&database, plan, json)?,
        Command::Quarantine { command } => run_quarantine(database, command)?,
        Command::Restore {
            entry,
            session,
            group,
            project,
            confirm,
        } => run_restore(database, entry, session, group, project, &confirm)?,
        Command::Recover { command } => run_recover(database, command)?,
        Command::Report { command } => run_report(database, command)?,
    }
    Ok(())
}

fn run_project(database: Database, command: ProjectCommand) -> anyhow::Result<()> {
    let repository = ProjectRepository::new(database);
    match command {
        ProjectCommand::Create { name, mode } => {
            let id = repository.create(&name, mode.into())?;
            println!("Đã tạo dự án: {id}");
        }
        ProjectCommand::AddRoot {
            project,
            path,
            primary,
        } => {
            let id = repository.add_root(project, &path, primary)?;
            println!("Đã thêm thư mục gốc: {id} ({})", path.display());
        }
        ProjectCommand::Update {
            project,
            name,
            mode,
        } => {
            repository.update(project, &name, mode.into())?;
            println!("Đã cập nhật dự án: {project}");
        }
        ProjectCommand::SetWorkers { project, workers } => {
            repository.set_worker_limit(project, workers)?;
            println!("Đã đặt giới hạn luồng của dự án {project} thành {workers}");
        }
        ProjectCommand::RemoveRoot { project, root } => {
            repository.remove_root(project, root)?;
            println!("Đã gỡ thư mục gốc khỏi cấu hình dự án: {root}");
        }
        ProjectCommand::Archive { project, confirm } => {
            require_exact(&confirm, "ARCHIVE")?;
            repository.archive(project)?;
            println!("Đã lưu trữ dự án mà không xóa dữ liệu nguồn: {project}");
        }
        ProjectCommand::List { json } => {
            let projects = repository.list()?;
            if json {
                println!("{}", serde_json::to_string_pretty(&projects)?);
            } else {
                for project in projects {
                    println!(
                        "{}\t{}\t{:?}\tworkers={}\t{}",
                        project.id,
                        project.name,
                        project.mode,
                        project.worker_limit,
                        project.status
                    );
                }
            }
        }
    }
    Ok(())
}

fn run_scan(database: Database, command: ScanCommand) -> anyhow::Result<()> {
    match command {
        ScanCommand::Start {
            project,
            mode,
            acknowledge_content_mode,
            all_files,
            minimum_size,
        } => run_scan_start(
            database,
            project,
            mode,
            acknowledge_content_mode,
            all_files,
            minimum_size,
        ),
        ScanCommand::Pause { session } => {
            ScanRepository::new(database).request_control(session, ScanControlRequest::Pause)?;
            println!("Đã yêu cầu tạm dừng tại ranh giới an toàn tiếp theo: {session}");
            Ok(())
        }
        ScanCommand::Resume { session } => {
            let scans = ScanRepository::new(database.clone());
            let state = scans.status(session)?.state;
            if matches!(state.as_str(), "pausing" | "paused") {
                scans.request_control(session, ScanControlRequest::Resume)?;
                println!("Đã yêu cầu tiếp tục: {session}");
                return Ok(());
            }
            if !matches!(state.as_str(), "interrupted" | "blocked") {
                return Err(DedupeError::Safety(format!(
                    "Phiên quét {session} không thể tiếp tục từ trạng thái {state}"
                ))
                .into());
            }
            let spec = scans.resume_spec(session)?;
            scans.prepare_resume(session)?;
            run_scan_session(
                database,
                spec.project_id,
                spec.session_id,
                spec.mode,
                spec.all_files,
                None,
            )
        }
        ScanCommand::Cancel { session } => {
            ScanRepository::new(database).request_control(session, ScanControlRequest::Cancel)?;
            println!("Đã yêu cầu hủy tại ranh giới an toàn tiếp theo: {session}");
            Ok(())
        }
        ScanCommand::Status { session, json } => {
            let status = ScanRepository::new(database).status(session)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&status)?);
            } else {
                println!("Phiên quét: {} ({})", status.id, status.state);
                println!(
                    "đã phát hiện: {} · đã xử lý: {} · đã bỏ qua: {} · không ổn định: {} · lỗi: {}",
                    status.discovered_files,
                    status.processed_files,
                    status.skipped,
                    status.unstable,
                    status.errors
                );
                println!(
                    "byte đã đọc: {} · nhóm: {} · byte có thể thu hồi: {}",
                    status.bytes_read, status.duplicate_groups, status.reclaimable_bytes
                );
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_scan_start(
    database: Database,
    project: Uuid,
    mode: Option<ModeArg>,
    acknowledge_content_mode: bool,
    all_files: bool,
    minimum_size: Option<u64>,
) -> anyhow::Result<()> {
    let projects = ProjectRepository::new(database.clone());
    let mode = mode.map_or(projects.mode(project)?, ComparisonMode::from);
    if mode == ComparisonMode::Content && !acknowledge_content_mode {
        return Err(DedupeError::Safety(
            "Chế độ chỉ so sánh nội dung bỏ qua tên tệp; hãy truyền --acknowledge-content-mode"
                .into(),
        )
        .into());
    }
    let scans = ScanRepository::new(database.clone());
    let session = scans.create_session_with_config(project, mode, all_files)?;
    eprintln!("Đã bắt đầu phiên quét: {session}");
    run_scan_session(database, project, session, mode, all_files, minimum_size)
}

#[allow(clippy::too_many_lines)]
fn run_scan_session(
    database: Database,
    project: Uuid,
    session: Uuid,
    mode: ComparisonMode,
    all_files: bool,
    minimum_size: Option<u64>,
) -> anyhow::Result<()> {
    let projects = ProjectRepository::new(database.clone());
    let roots = projects.roots(project)?;
    if roots.is_empty() {
        return Err(
            DedupeError::InvalidInput("Dự án không có thư mục nguồn đang bật".into()).into(),
        );
    }
    let source_paths = roots
        .iter()
        .map(|(_, path, _)| path.clone())
        .collect::<Vec<_>>();
    let filter = configured_scan_filter(&projects, project, all_files, minimum_size)?;
    let workers = projects.worker_config(project)?;
    let mut sink = SqliteScanSink::new(
        database.clone(),
        project,
        session,
        roots
            .iter()
            .map(|(id, path, _)| (*id, path.clone()))
            .collect(),
    );
    let provider = PlatformFileSystem;
    let control = ControlToken::new();
    let progress = Arc::new(ProgressCounters::default());
    let monitor = ScanControlMonitor::start(
        database.clone(),
        session,
        control.clone(),
        Arc::clone(&progress),
    );
    let scans = ScanRepository::new(database.clone());
    let scan_result = (|| -> dedupe_core::Result<Vec<DuplicateGroup>> {
        scan_roots_with_config(
            &source_paths,
            &filter,
            &provider,
            &mut sink,
            &control,
            progress.as_ref(),
            workers,
        )?;
        let enumeration = progress.snapshot();
        scans.checkpoint(session, "metadata_complete", enumeration.processed_files)?;
        scans.update_progress(session, enumeration)?;
        scans.set_state(session, "quick_hashing")?;
        let primary_roots = roots
            .iter()
            .filter(|(_, _, primary)| *primary)
            .map(|(_, path, _)| path.clone())
            .collect::<Vec<_>>();
        let policy = KeepPolicy::Default { primary_roots };
        let mut groups = Vec::new();
        scans.for_each_candidate_group(session, mode, |candidates| {
            control.checkpoint()?;
            let mut outcome = confirm_preliminary_group_detailed_with_config(
                mode,
                &candidates,
                &provider,
                &control,
                workers,
            )?;
            progress.add_bytes(outcome.bytes_read);
            progress.add_unstable(outcome.unstable_files);
            progress.add_errors(outcome.errors.len() as u64);
            for error in outcome.errors {
                eprintln!(
                    "Đã bỏ qua tệp an toàn trong bước {} đối với {}: {}",
                    error.stage,
                    error.path.display(),
                    error.error
                );
                sink.record_error(&error.path, &error.error)?;
            }
            for group in &mut outcome.groups {
                keep_policy::apply(group, &policy)?;
            }
            groups.extend(outcome.groups);
            Ok(())
        })?;
        sink.flush()?;
        scans.checkpoint(session, "hashing_complete", groups.len() as u64)?;
        scans.set_state(session, "grouping")?;
        DuplicateRepository::new(database.clone()).replace_session_groups(session, &groups)?;
        Ok(groups)
    })();
    let monitor_result = monitor.finish();
    let groups = match (scan_result, monitor_result) {
        (Ok(groups), Ok(())) => groups,
        (Err(error), _) if matches!(error, DedupeError::Cancelled) => {
            let snapshot = progress.snapshot();
            let _ = scans.update_progress(session, snapshot);
            let _ = scans.set_state(session, "cancelled");
            return Err(error.into());
        }
        (Err(error), _) | (Ok(_), Err(error)) => {
            let _ = scans.update_progress(session, progress.snapshot());
            let _ = scans.block_session(session, &error.to_string());
            return Err(error.into());
        }
    };
    let snapshot = progress.snapshot();
    scans.complete_session(session, snapshot)?;
    print_completed_scan(session, snapshot, &groups)?;
    Ok(())
}

fn configured_scan_filter(
    projects: &ProjectRepository,
    project: Uuid,
    all_files: bool,
    minimum_size: Option<u64>,
) -> dedupe_core::Result<CompiledFilter> {
    let mut config = projects.filter_config(project)?;
    if all_files {
        config.include_extensions.clear();
    }
    if let Some(minimum_size) = minimum_size {
        config.minimum_size = minimum_size;
    }
    CompiledFilter::new(config)
}

fn print_completed_scan(
    session: Uuid,
    snapshot: dedupe_core::progress::ProgressSnapshot,
    groups: &[DuplicateGroup],
) -> anyhow::Result<()> {
    let result = ScanResult {
        session_id: session,
        discovered_files: snapshot.discovered_files,
        processed_files: snapshot.processed_files,
        bytes_read: snapshot.bytes_read,
        errors: snapshot.errors,
        skipped: snapshot.skipped,
        unstable: snapshot.unstable,
        duplicate_groups: groups.len(),
        reclaimable_bytes: groups
            .iter()
            .map(DuplicateGroup::maximum_reclaimable_bytes)
            .fold(0_u64, u64::saturating_add),
    };
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn run_results(database: Database, command: ResultsCommand) -> anyhow::Result<()> {
    let ResultsCommand::List { session, json } = command;
    let groups = DuplicateRepository::new(database).load_session_groups(session)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&groups)?);
    } else if groups.is_empty() {
        println!("Không có nhóm trùng lặp đã chứng minh");
    } else {
        print_groups(&groups);
    }
    Ok(())
}

fn run_plan(database: Database, command: PlanCommand) -> anyhow::Result<()> {
    match command {
        PlanCommand::Create { session, policy } => {
            let duplicates = DuplicateRepository::new(database.clone());
            let mut groups = duplicates.load_session_groups(session)?;
            let primary_roots = project_primary_roots_for_session(&database, session)?;
            let policy = match policy {
                PolicyArg::Default => KeepPolicy::Default { primary_roots },
                PolicyArg::Oldest => KeepPolicy::Oldest,
                PolicyArg::Newest => KeepPolicy::Newest,
                PolicyArg::Shortest => KeepPolicy::ShortestPath,
            };
            for group in &mut groups {
                keep_policy::apply(group, &policy)?;
            }
            duplicates.replace_session_groups(session, &groups)?;
            let id = PlanRepository::new(database).create_and_seal(session, &policy, &groups)?;
            println!("Đã tạo kế hoạch khóa: {id}");
        }
        PlanCommand::Validate { plan, json } => print_plan_summary(&database, plan, json)?,
    }
    Ok(())
}

fn print_plan_summary(database: &Database, plan: Uuid, json: bool) -> anyhow::Result<()> {
    let plans = PlanRepository::new(database.clone());
    let summary = plans.summary(plan)?;
    if summary.status != "sealed" && summary.status != "executing" && summary.status != "completed"
    {
        return Err(DedupeError::Safety(format!(
            "Kế hoạch {} không hợp lệ để xem xét",
            summary.plan_id
        ))
        .into());
    }
    if summary.status == "sealed" {
        let groups =
            DuplicateRepository::new(database.clone()).load_session_groups(summary.session_id)?;
        if let Err(error) = dedupe_core::dry_run::validate_fresh(&groups, &PlatformFileSystem) {
            plans.mark_stale(plan)?;
            return Err(error.into());
        }
    }
    if json {
        println!("{}", serde_json::to_string_pretty(&summary)?);
    } else {
        println!("Kế hoạch: {} ({})", summary.plan_id, summary.status);
        println!("Số nhóm: {}", summary.groups);
        println!("Số tệp đề xuất cách ly: {}", summary.quarantine_files);
        println!("Số byte đề xuất cách ly: {}", summary.quarantine_bytes);
        println!("Chỉ chạy thử: hệ thống tệp không bị thay đổi");
    }
    Ok(())
}

fn run_quarantine(database: Database, command: QuarantineCommand) -> anyhow::Result<()> {
    match command {
        QuarantineCommand::List { project, json } => {
            let entries = TransactionRepository::new(database).list_quarantine(project)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&entries)?);
            } else {
                for entry in entries {
                    println!(
                        "{}\t{}\t{} bytes\t{} -> {}",
                        entry.id,
                        entry.state,
                        entry.size_bytes,
                        entry.original_path.display(),
                        entry.quarantine_path.display()
                    );
                }
            }
        }
        QuarantineCommand::Apply {
            plan,
            confirm,
            quarantine_root,
        } => {
            require_exact(&confirm, "QUARANTINE")?;
            apply_quarantine_plan(database, plan, quarantine_root.as_deref())?;
        }
        QuarantineCommand::DeletePrepare {
            entries,
            delete_now,
            json,
        } => {
            let selected =
                PermanentDeleteRepository::new(database.clone()).selected_entries(&entries)?;
            let journal = SqlitePermanentDeleteJournal::new(
                database.clone(),
                permanent_delete_manifest_path(&database),
            )?;
            let challenge = if delete_now {
                permanent_delete::prepare_immediate(selected, &journal, chrono::Utc::now())?
            } else {
                permanent_delete::prepare(selected, &journal, chrono::Utc::now())?
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&challenge)?);
            } else {
                println!("Lô: {}", challenge.batch_id);
                println!("Số tệp đã chọn: {}", challenge.entry_count);
                println!("Số byte đã chọn: {}", challenge.total_bytes);
                println!("Hết hạn lúc: {}", challenge.expires_at);
                println!("Token: {}", challenge.token);
                println!("Nhập chính xác: {}", challenge.confirmation_phrase);
                println!("Chưa có tệp nào bị xóa; hãy chủ động chạy quarantine delete-execute");
            }
        }
        QuarantineCommand::DeleteExecute {
            batch,
            token,
            confirm,
            json,
        } => {
            let journal = SqlitePermanentDeleteJournal::new(
                database.clone(),
                permanent_delete_manifest_path(&database),
            )?;
            let provider = PlatformFileSystem;
            let outcome = permanent_delete::execute(
                batch,
                &token,
                &confirm,
                &journal,
                &provider,
                &provider,
                &ControlToken::new(),
                chrono::Utc::now(),
            )?;
            if json {
                println!("{}", serde_json::to_string_pretty(&outcome)?);
            } else {
                println!("Số mục đã xóa vĩnh viễn: {}", outcome.deleted_entries);
                println!("Số byte đã xóa vĩnh viễn: {}", outcome.deleted_bytes);
            }
        }
    }
    Ok(())
}

fn apply_quarantine_plan(
    database: Database,
    plan: Uuid,
    explicit_quarantine_root: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    let plans = PlanRepository::new(database.clone());
    let summary = plans.summary(plan)?;
    let items = plans.quarantine_items(plan)?;
    let project_id = plans.project_id(plan)?;
    let roots = ProjectRepository::new(database.clone()).roots(project_id)?;
    let groups =
        DuplicateRepository::new(database.clone()).load_session_groups(summary.session_id)?;
    for group in &groups {
        quarantine::verify_live_keeper(group, &PlatformFileSystem, &ControlToken::new())?;
    }
    plans.mark_executing(plan)?;
    let journal = SqliteTransactionJournal::new(database.clone(), manifest_path(&database))?;
    let control = ControlToken::new();
    let provider = PlatformFileSystem;
    let mut verified_bytes = 0_u64;
    for item in items {
        let group = groups
            .iter()
            .find(|group| group.id == item.group_id)
            .ok_or_else(|| {
                DedupeError::State("Kế hoạch tham chiếu đến nhóm không tồn tại".into())
            })?;
        quarantine::verify_live_keeper(group, &provider, &ControlToken::new())?;
        let source_root = roots
            .iter()
            .filter(|(_, root, _)| item.file.metadata.path.starts_with(root))
            .max_by_key(|(_, root, _)| root.as_os_str().len())
            .map(|(_, root, _)| root)
            .ok_or_else(|| {
                DedupeError::Safety(
                    "Nguồn trong kế hoạch nằm ngoài các thư mục gốc đã cấu hình".into(),
                )
            })?;
        let quarantine_root = explicit_quarantine_root.map_or_else(
            || source_root.join(".safe-duplicate-finder-quarantine"),
            PathBuf::from,
        );
        let destination = quarantine::quarantine_destination(
            &quarantine_root,
            project_id,
            summary.session_id,
            item.plan_item_id,
            source_root,
            &item.file.metadata.path,
        )?;
        let mut transaction = quarantine::planned_transaction(
            project_id,
            summary.session_id,
            item.plan_item_id,
            &item.file,
            destination,
        )?;
        quarantine::execute(&mut transaction, &provider, &provider, &journal, &control)?;
        verified_bytes = verified_bytes.saturating_add(item.file.metadata.size_bytes);
        println!(
            "Đã xác minh cách ly: {} ({} byte)",
            item.file.metadata.path.display(),
            item.file.metadata.size_bytes
        );
    }
    if !plans.mark_completed_if_verified(plan)? {
        return Err(DedupeError::State("Kế hoạch còn mục cách ly chưa xác minh".into()).into());
    }
    println!("Số byte cách ly đã xác minh: {verified_bytes}");
    Ok(())
}

fn run_restore(
    database: Database,
    entry: Option<Uuid>,
    session: Option<Uuid>,
    group: Option<Uuid>,
    project: Option<Uuid>,
    confirm: &str,
) -> anyhow::Result<()> {
    require_exact(confirm, "RESTORE")?;
    let repository = TransactionRepository::new(database.clone());
    let entries = if let Some(entry) = entry {
        vec![entry]
    } else if let Some(session) = session {
        repository.verified_entries_for_session(session)?
    } else if let Some(group) = group {
        repository.verified_entries_for_group(group)?
    } else if let Some(project) = project {
        repository.verified_entries_for_project(project)?
    } else {
        return Err(DedupeError::InvalidInput("Phải chỉ định phạm vi khôi phục".into()).into());
    };
    if entries.is_empty() {
        println!("Không còn mục cách ly đã xác minh trong phạm vi đã chọn");
        return Ok(());
    }
    let selected = entries.len();
    for entry in entries {
        let destination = restore_one(&database, &repository, entry)?;
        println!("Đã xác minh khôi phục: {}", destination.display());
    }
    println!("Đã hoàn tất lô khôi phục: {selected} mục được xác minh");
    Ok(())
}

fn restore_one(
    database: &Database,
    repository: &TransactionRepository,
    entry: Uuid,
) -> anyhow::Result<PathBuf> {
    let origin = repository.verified_quarantine_transaction(entry)?;
    let mut restore = dedupe_core::restore::planned_transaction(&origin)?;
    let journal = SqliteTransactionJournal::new(database.clone(), manifest_path(database))?;
    let provider = PlatformFileSystem;
    dedupe_core::restore::execute(
        &mut restore,
        &provider,
        &provider,
        &journal,
        &ControlToken::new(),
    )?;
    Ok(restore.destination)
}

fn run_recover(database: Database, command: RecoverCommand) -> anyhow::Result<()> {
    let repository = TransactionRepository::new(database.clone());
    match command {
        RecoverCommand::Inspect { project, json } => {
            let transactions = repository.pending_recovery(project)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&transactions)?);
            } else if transactions.is_empty() {
                println!("Không có giao dịch bị gián đoạn");
            } else {
                for transaction in transactions {
                    println!(
                        "{}\t{:?}\t{} -> {}",
                        transaction.id,
                        transaction.state,
                        transaction.source.display(),
                        transaction.destination.display()
                    );
                }
            }
        }
        RecoverCommand::Reconcile {
            transaction,
            confirm,
        } => {
            require_exact(&confirm, "RECONCILE")?;
            let mut transaction = repository.transaction(transaction)?;
            let journal =
                SqliteTransactionJournal::new(database.clone(), manifest_path(&database))?;
            let outcome = dedupe_core::recovery::reconcile(
                &mut transaction,
                &PlatformFileSystem,
                &journal,
                &ControlToken::new(),
            )?;
            println!("Kết quả đối soát: {outcome:?}");
        }
    }
    Ok(())
}

fn run_report(database: Database, command: ReportCommand) -> anyhow::Result<()> {
    let ReportCommand::Export {
        session,
        format,
        destination,
    } = command;
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Tạo thư mục báo cáo {}", parent.display()))?;
    }
    let groups = DuplicateRepository::new(database).load_session_groups(session)?;
    let file = File::create(&destination)
        .with_context(|| format!("Tạo báo cáo {}", destination.display()))?;
    match format {
        ReportFormat::Csv => dedupe_report::write_csv(&groups, file)?,
        ReportFormat::Json => dedupe_report::write_json(&groups, file)?,
        ReportFormat::Html => dedupe_report::write_html(&groups, file)?,
    }
    println!("Đã ghi báo cáo: {}", destination.display());
    Ok(())
}

fn project_primary_roots_for_session(
    database: &Database,
    session: Uuid,
) -> anyhow::Result<Vec<PathBuf>> {
    let project: String = database.connection().query_row(
        "SELECT project_id FROM scan_sessions WHERE id=?1",
        [session.to_string()],
        |row| row.get(0),
    )?;
    let project = Uuid::parse_str(&project).context("UUID dự án đã lưu không hợp lệ")?;
    Ok(ProjectRepository::new(database.clone())
        .roots(project)?
        .into_iter()
        .filter(|(_, _, primary)| *primary)
        .map(|(_, path, _)| path)
        .collect())
}

fn print_groups(groups: &[DuplicateGroup]) {
    for group in groups {
        println!(
            "Nhóm {}: {} thành viên, mỗi tệp {} byte, có thể thu hồi {} byte",
            group.id,
            group.members.len(),
            group.size_bytes,
            group.maximum_reclaimable_bytes()
        );
        for member in &group.members {
            println!(
                "  {:?}\t{}\t{}",
                member.action,
                member.file.metadata.path.display(),
                member.reason
            );
        }
    }
}

fn require_exact(actual: &str, required: &str) -> anyhow::Result<()> {
    if actual == required {
        Ok(())
    } else {
        Err(DedupeError::Safety(format!("Xác nhận phải chính xác là {required}")).into())
    }
}

fn manifest_path(database: &Database) -> PathBuf {
    database.path().with_extension("transactions.jsonl")
}

fn permanent_delete_manifest_path(database: &Database) -> PathBuf {
    database.path().with_extension("permanent-delete.jsonl")
}

fn exit_code(error: &anyhow::Error) -> u8 {
    if let Some(error) = error.downcast_ref::<DedupeError>() {
        return match error {
            DedupeError::InvalidInput(_) => 2,
            DedupeError::Safety(_) => 3,
            DedupeError::Durability(_) | DedupeError::State(_) => 10,
            DedupeError::Cancelled => 5,
            DedupeError::Io { .. } | DedupeError::Serialization(_) => 20,
        };
    }
    if error
        .downcast_ref::<dedupe_store::database::StoreError>()
        .is_some()
    {
        10
    } else {
        20
    }
}

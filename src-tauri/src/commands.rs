//! One-line Tauri adapters over the Office. Argument names are the contract's (camelCase in JS).

use crate::backup;
use crate::model::*;
use crate::office::Office;
use crate::update;
use std::path::Path;
use tauri::{AppHandle, Manager, State};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_dialog::DialogExt;
use tauri_plugin_opener::OpenerExt;

type O<'a> = State<'a, Office>;

fn background(app: &AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
pub async fn setup_status(app: AppHandle, office: O<'_>) -> Result<SetupStatus> {
    office.setup_status(background(&app)).await
}

#[tauri::command]
pub async fn complete_setup(office: O<'_>, company_name: String, ceo_name: String, title: Option<String>) -> Result<Company> {
    office.complete_setup(company_name, ceo_name, title).await
}

#[tauri::command]
pub async fn update_company(office: O<'_>, name: String, ceo_name: String, title: String, reports_to: Option<String>) -> Result<Company> {
    office.update_company(name, ceo_name, title, reports_to)
}

#[tauri::command]
pub async fn set_background(app: AppHandle, office: O<'_>, enabled: bool) -> Result<SetupStatus> {
    let al = app.autolaunch();
    if enabled { al.enable() } else { al.disable() }.map_err(|e| AppError::internal(format!("autostart: {e}")))?;
    office.setup_status(background(&app)).await
}

#[tauri::command]
pub async fn org(office: O<'_>) -> Result<OrgSnapshot> {
    office.org()
}

#[tauri::command]
pub async fn skill_catalog(office: O<'_>) -> Result<Vec<SkillInfo>> {
    Ok(office.skill_catalog())
}

#[tauri::command]
pub async fn skill_sources(office: O<'_>) -> Result<SkillSources> {
    Ok(office.skill_sources())
}

#[tauri::command]
pub async fn hire(office: O<'_>, draft: EmployeeDraft, imports: Option<Vec<String>>) -> Result<Employee> {
    office.hire(draft, imports.unwrap_or_default())
}

#[tauri::command]
pub async fn update_employee(office: O<'_>, employee: Employee) -> Result<Employee> {
    office.update_employee(employee)
}

#[tauri::command]
pub async fn let_go(office: O<'_>, employee_id: String) -> Result<()> {
    office.let_go(&employee_id)
}

#[tauri::command]
pub async fn schedules(office: O<'_>, employee_id: String) -> Result<Vec<Schedule>> {
    office.schedules(&employee_id)
}

#[tauri::command]
pub async fn save_schedule(office: O<'_>, draft: ScheduleDraft) -> Result<Schedule> {
    office.save_schedule(draft)
}

#[tauri::command]
pub async fn delete_schedule(office: O<'_>, schedule_id: String) -> Result<()> {
    office.delete_schedule(&schedule_id)
}

#[tauri::command]
pub async fn cron_preview(office: O<'_>, cron: String) -> Result<CronPreview> {
    Ok(office.cron_preview(&cron))
}

#[tauri::command]
pub async fn run_now(office: O<'_>, employee_id: String, instruction: Option<String>) -> Result<Run> {
    office.run_now(&employee_id, instruction)
}

#[tauri::command]
pub async fn cancel_run(office: O<'_>, run_id: String) -> Result<()> {
    office.cancel_run(&run_id)
}

#[tauri::command]
pub async fn send_chat(office: O<'_>, employee_id: String, text: String, attachments: Vec<NewAttachment>) -> Result<Run> {
    office.send_chat(&employee_id, &text, attachments)
}

#[tauri::command]
pub async fn start_task(office: O<'_>, employee_id: String, text: String, attachments: Vec<NewAttachment>) -> Result<Run> {
    office.start_task(&employee_id, &text, attachments)
}

#[tauri::command]
pub async fn open_attachment(app: AppHandle, office: O<'_>, run_id: String, name: String) -> Result<()> {
    let path = office.attachment_path(&run_id, &name)?.to_string_lossy().into_owned();
    app.opener().open_path(path, None::<&str>).map_err(|e| AppError::internal(format!("Could not open {name}: {e}")))
}

const MAX_IMAGE_BYTES: u64 = 25 * 1024 * 1024;

/// The MIME type of a local image an agent mentioned by its absolute path.
fn image_mime(path: &str) -> Result<&'static str> {
    let p = Path::new(path);
    let mime = match p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
        Some("png") => Some("image/png"),
        Some("jpg" | "jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        _ => None,
    };
    mime.filter(|_| p.is_absolute()).ok_or_else(|| AppError::invalid(format!("{path} is not a local image")))
}

#[tauri::command]
pub async fn read_image(path: String) -> Result<String> {
    use base64::Engine;
    let mime = image_mime(&path)?;
    let meta = std::fs::metadata(&path).map_err(|_| AppError::not_found(format!("No image at {path}")))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(AppError::invalid(format!("{path} is larger than {} MB", MAX_IMAGE_BYTES / (1024 * 1024))));
    }
    Ok(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(std::fs::read(&path)?)))
}

#[tauri::command]
pub async fn open_image(app: AppHandle, path: String) -> Result<()> {
    image_mime(&path)?;
    app.opener().open_path(&path, None::<&str>).map_err(|e| AppError::internal(format!("Could not open {path}: {e}")))
}

/// Kinds of file a click on a report path opens in their app. Anything else is shown in Finder.
const OPENS: &[&str] = &[
    "pdf", "png", "jpg", "jpeg", "gif", "webp", "svg", "mp4", "mov", "m4a", "mp3", "wav", "txt", "md", "csv", "html", "docx", "xlsx",
    "pptx", "key", "pages", "numbers",
];

/// Whether a report path opens in its app, so a click never runs a program
/// (an .app bundle is a folder, a .command or .sh is a file).
fn opens_directly(path: &Path, is_dir: bool) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => OPENS.contains(&ext.to_ascii_lowercase().as_str()),
        None => is_dir,
    }
}

/// A path from a report as a full path, with a leading `~/` expanded.
fn report_path(path: &str) -> Result<std::path::PathBuf> {
    let full = match path.strip_prefix("~/") {
        Some(rest) => dirs::home_dir().map(|home| home.join(rest)),
        None => Some(std::path::PathBuf::from(path)),
    };
    full.filter(|p| p.is_absolute()).ok_or_else(|| AppError::invalid(format!("{path} is not a full path")))
}

#[tauri::command]
pub async fn open_report_path(app: AppHandle, path: String) -> Result<()> {
    // Resolved, so a link named like a document cannot point at a program.
    let full = std::fs::canonicalize(report_path(&path)?).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => AppError::not_found(format!("Nothing is at {path} any more")),
        _ => AppError::internal(format!("Could not open {path}: {e}")),
    })?;
    let done = if opens_directly(&full, full.is_dir()) {
        app.opener().open_path(full.to_string_lossy(), None::<&str>)
    } else {
        app.opener().reveal_item_in_dir(&full)
    };
    done.map_err(|e| AppError::internal(format!("Could not open {path}: {e}")))
}

#[tauri::command]
pub async fn new_chat(office: O<'_>, employee_id: String) -> Result<()> {
    office.new_chat(&employee_id)
}

#[tauri::command]
pub async fn chat(office: O<'_>, employee_id: String) -> Result<Vec<Run>> {
    office.chat(&employee_id)
}

#[tauri::command]
pub async fn runs(office: O<'_>, employee_id: Option<String>, limit: u32) -> Result<Vec<Run>> {
    office.runs(employee_id.as_deref(), limit)
}

#[tauri::command]
pub async fn run_detail(office: O<'_>, run_id: String) -> Result<RunDetail> {
    office.run_detail(&run_id)
}

#[tauri::command]
pub async fn archived(office: O<'_>) -> Result<Vec<Thread>> {
    office.archived()
}

#[tauri::command]
pub async fn conversations(office: O<'_>) -> Result<Conversations> {
    office.conversations()
}

#[tauri::command]
pub async fn thread(office: O<'_>, run_id: String) -> Result<Thread> {
    office.thread(&run_id)
}

#[tauri::command]
pub async fn chat_briefs(office: O<'_>, employee_id: String) -> Result<Vec<Brief>> {
    office.chat_briefs(&employee_id)
}

#[tauri::command]
pub async fn set_archived(office: O<'_>, run_id: String, archived: bool, brief: Option<bool>) -> Result<Run> {
    office.set_archived(&run_id, archived, brief)
}

#[tauri::command]
pub async fn reply_to_run(office: O<'_>, run_id: String, text: String, archive: bool, attachments: Option<Vec<NewAttachment>>) -> Result<Run> {
    office.reply_to_run(&run_id, &text, archive, attachments.unwrap_or_default())
}

#[tauri::command]
pub async fn approve_blocked(office: O<'_>, run_id: String, rules: Vec<String>, always: bool) -> Result<Run> {
    office.approve_blocked(&run_id, rules, always)
}

#[tauri::command]
pub async fn open_asks(office: O<'_>) -> Result<Vec<Run>> {
    office.open_asks()
}

#[tauri::command]
pub async fn employee_skills(office: O<'_>, employee_id: String) -> Result<Vec<OwnSkill>> {
    office.employee_skills(&employee_id)
}

#[tauri::command]
pub async fn skill_health(office: O<'_>, employee_id: String) -> Result<SkillHealth> {
    office.skill_health(&employee_id)
}

#[tauri::command]
pub async fn tidy(office: O<'_>, employee_id: String) -> Result<Run> {
    office.tidy(&employee_id)
}

#[tauri::command]
pub async fn approve_tidy(office: O<'_>, run_id: String) -> Result<Run> {
    office.approve_tidy(&run_id)
}

#[tauri::command]
pub async fn tidy_runs(office: O<'_>, employee_id: String) -> Result<Vec<Run>> {
    office.tidy_runs(&employee_id)
}

#[tauri::command]
pub async fn checks(office: O<'_>, employee_id: String) -> Result<Vec<Check>> {
    office.checks(&employee_id)
}

#[tauri::command]
pub async fn drop_check(office: O<'_>, check_id: String) -> Result<Check> {
    office.drop_check(&check_id)
}

#[tauri::command]
pub async fn read_skill_file(office: O<'_>, employee_id: String, skill: String, file: String) -> Result<String> {
    office.read_skill_file(&employee_id, &skill, &file)
}

#[tauri::command]
pub async fn save_skill_file(office: O<'_>, employee_id: String, skill: String, file: String, text: String) -> Result<Option<SkillRevision>> {
    office.save_skill_file(&employee_id, &skill, &file, &text)
}

#[tauri::command]
pub async fn skill_history(office: O<'_>, employee_id: String) -> Result<Vec<SkillRevision>> {
    office.skill_history(&employee_id)
}

#[tauri::command]
pub async fn revert_revision(office: O<'_>, revision_id: String) -> Result<SkillRevision> {
    office.revert_revision(&revision_id)
}

#[tauri::command]
pub async fn promote_skill(office: O<'_>, employee_id: String, skill: String) -> Result<SkillInfo> {
    office.promote_skill(&employee_id, &skill)
}

#[tauri::command]
pub async fn import_skills(office: O<'_>, employee_id: String, skill_ids: Vec<String>) -> Result<Run> {
    office.import_skills(&employee_id, &skill_ids)
}

#[tauri::command]
pub async fn staged_imports(office: O<'_>, employee_id: String) -> Result<Vec<StagedImport>> {
    office.staged_imports(&employee_id)
}

#[tauri::command]
pub async fn organize(office: O<'_>, employee_id: String) -> Result<Run> {
    office.organize(&employee_id)
}

#[tauri::command]
pub async fn revert_run(office: O<'_>, run_id: String) -> Result<u32> {
    office.revert_run(&run_id)
}

#[tauri::command]
pub async fn import_originals(office: O<'_>, run_id: String) -> Result<Vec<OriginalSkill>> {
    office.import_originals(&run_id)
}

#[tauri::command]
pub async fn remove_originals(office: O<'_>, run_id: String, names: Vec<String>) -> Result<Vec<String>> {
    office.remove_originals(&run_id, &names)
}

#[tauri::command]
pub async fn settle_originals(office: O<'_>, run_id: String) -> Result<Run> {
    office.settle_originals(&run_id)
}

#[tauri::command]
pub async fn allow_rule(office: O<'_>, employee_id: String, rule: String) -> Result<Employee> {
    office.allow_rule(&employee_id, &rule)
}

#[tauri::command]
pub async fn add_person(office: O<'_>, draft: PersonDraft) -> Result<Person> {
    office.add_person(draft)
}

#[tauri::command]
pub async fn update_person(office: O<'_>, person: Person) -> Result<Person> {
    office.update_person(person)
}

#[tauri::command]
pub async fn remove_person(office: O<'_>, person_id: String) -> Result<()> {
    office.remove_person(&person_id)
}

#[tauri::command]
pub async fn requests(office: O<'_>, person_id: Option<String>, employee_id: Option<String>, state: Option<String>) -> Result<Vec<HumanRequest>> {
    office.requests(person_id, employee_id, state)
}

#[tauri::command]
pub async fn resolve_request(office: O<'_>, request_id: String, state: String, response: Option<String>) -> Result<HumanRequest> {
    office.resolve_request(&request_id, &state, response, None)
}

#[tauri::command]
pub async fn watch_request(office: O<'_>, request_id: String, action: WatchAction) -> Result<HumanRequest> {
    office.watch_request(&request_id, action).await
}

#[tauri::command]
pub async fn proposals(office: O<'_>, state: Option<String>) -> Result<Vec<HireProposal>> {
    office.proposals(state)
}

#[tauri::command]
pub async fn approve_proposal(
    office: O<'_>,
    proposal_id: String,
    draft: EmployeeDraft,
    imports: Option<Vec<String>>,
    note: Option<String>,
) -> Result<Employee> {
    office.approve_proposal(&proposal_id, draft, imports.unwrap_or_default(), note)
}

#[tauri::command]
pub async fn decline_proposal(office: O<'_>, proposal_id: String, note: Option<String>) -> Result<HireProposal> {
    office.decline_proposal(&proposal_id, note)
}

#[tauri::command]
pub async fn settings(office: O<'_>) -> Result<AppSettings> {
    office.settings()
}

#[tauri::command]
pub async fn save_settings(office: O<'_>, settings: AppSettings) -> Result<AppSettings> {
    office.save_settings(settings)
}

/// Rust-side, so the webview's opener ACL can stay limited to URLs.
#[tauri::command]
pub async fn open_data_dir(app: AppHandle, office: O<'_>) -> Result<()> {
    let dir = office.data_dir().to_string_lossy().into_owned();
    app.opener().open_path(dir, None::<&str>).map_err(|e| AppError::internal(format!("Could not open the office folder: {e}")))
}

#[tauri::command]
pub async fn backup_status(office: O<'_>) -> Result<BackupStatus> {
    office.backup_status()
}

#[tauri::command]
pub async fn backup_now(office: O<'_>) -> Result<BackupStatus> {
    office.backup_now().await
}

/// A native folder picker, run from Rust so the webview needs no dialog or path permissions.
/// `None` when cancelled.
#[tauri::command]
pub async fn pick_backup_dir(app: AppHandle, office: O<'_>) -> Result<Option<String>> {
    let current = office.settings()?.backup.dir;
    let mut dialog = app.dialog().file().set_title("Choose a folder for backups");
    if Path::new(&current).is_dir() {
        dialog = dialog.set_directory(current);
    }
    if let Some(w) = app.get_webview_window("main") {
        dialog = dialog.set_parent(&w);
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    dialog.pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    match rx.await.ok().flatten() {
        None => Ok(None),
        Some(p) => {
            let p = p.into_path().map_err(|e| AppError::internal(format!("Could not use that folder: {e}")))?;
            Ok(Some(p.to_string_lossy().into_owned()))
        }
    }
}

#[tauri::command]
pub async fn open_backup_dir(app: AppHandle, office: O<'_>) -> Result<()> {
    let dir = backup::validate_dir(&office.settings()?.backup.dir, office.data_dir())?;
    std::fs::create_dir_all(&dir)?;
    app.opener().open_path(dir.to_string_lossy(), None::<&str>).map_err(|e| AppError::internal(format!("Could not open the backup folder: {e}")))
}

#[tauri::command]
pub async fn usage_limits(office: O<'_>) -> Result<UsageLimits> {
    Ok(office.usage_limits())
}

#[tauri::command]
pub async fn refresh_usage(office: O<'_>) -> Result<UsageLimits> {
    Ok(office.refresh_usage().await)
}

#[tauri::command]
pub async fn use_signed_in_account(office: O<'_>) -> Result<Company> {
    office.use_signed_in_account().await
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<String>> {
    update::check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<()> {
    update::install(&app).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn documents_and_folders_open_programs_are_only_shown() {
        assert!(opens_directly(Path::new("/tmp/out/summary.pdf"), false));
        assert!(opens_directly(Path::new("/tmp/out/Summary.PDF"), false));
        assert!(opens_directly(Path::new("/tmp/out/reports"), true));
        assert!(!opens_directly(Path::new("/tmp/out/setup.sh"), false));
        assert!(!opens_directly(Path::new("/tmp/out/X.app"), true));
        assert!(!opens_directly(Path::new("/tmp/out/start.command"), false));
        assert!(!opens_directly(Path::new("/tmp/out/tool"), false));
    }

    #[test]
    fn report_paths_must_be_full() {
        assert_eq!(report_path("out/summary.pdf").unwrap_err().message, "out/summary.pdf is not a full path");
        assert_eq!(report_path("/tmp/summary.pdf").unwrap(), PathBuf::from("/tmp/summary.pdf"));
        if let Some(home) = dirs::home_dir() {
            assert_eq!(report_path("~/summary.pdf").unwrap(), home.join("summary.pdf"));
        }
    }
}

// The office tools' schemas (`mcp::tools`) are one `json!` literal, deeper than the default limit.
#![recursion_limit = "256"]

mod backup;
mod cards;
mod catalog;
mod claude;
mod commands;
mod error;
mod health;
mod log;
mod mcp;
mod model;
mod notices;
mod office;
mod org;
mod own_skills;
mod power;
mod proxy;
mod reader;
mod schedule;
mod store;
mod threads;
mod tidy;
mod update;
mod watch;

use log::log;
use model::{AppSettings, BackupStatus, Build, HireProposal, HumanRequest, Run, RunEvent, UsageLimits};
use notices::Notices;
use office::{EventSink, Office};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Instant;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

struct TauriSink {
    app: AppHandle,
    pause_item: OnceLock<MenuItem<Wry>>,
    /// Notifications waiting to show, as the notification settings say (`notices`).
    notices: Mutex<Notices>,
}

impl TauriSink {
    fn notices(&self) -> MutexGuard<'_, Notices> {
        self.notices.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn show(&self, title: &str, body: &str) {
        if let Err(e) = self.app.notification().builder().title(title).body(body).show() {
            log!("notification failed: {e}");
        }
    }
}

/// Minutes since midnight on this computer's clock, which quiet hours are set in.
fn local_minute() -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

/// Shows the notifications that come due, every `notices::TICK` for as long as the app runs.
fn spawn_notices(sink: Arc<TauriSink>) {
    tauri::async_runtime::spawn(async move {
        let mut every = tokio::time::interval(notices::TICK);
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            every.tick().await;
            let due = sink.notices().due(Instant::now(), local_minute());
            for (title, body) in due {
                sink.show(&title, &body);
            }
        }
    });
}

fn pause_label(paused: bool) -> &'static str {
    if paused { "Resume schedules" } else { "Pause schedules" }
}

impl EventSink for TauriSink {
    fn org_changed(&self) {
        let _ = self.app.emit("org:changed", ());
    }
    fn run_updated(&self, run: &Run) {
        let _ = self.app.emit("run:updated", run);
    }
    fn run_events(&self, run_id: &str, events: &[RunEvent]) {
        let _ = self.app.emit("run:event", serde_json::json!({ "runId": run_id, "events": events }));
    }
    fn request_updated(&self, request: &HumanRequest) {
        let _ = self.app.emit("request:updated", request);
    }
    fn proposal_updated(&self, proposal: &HireProposal) {
        let _ = self.app.emit("proposal:updated", proposal);
    }
    fn usage_updated(&self, usage: &UsageLimits) {
        let _ = self.app.emit("usage:updated", usage);
    }
    fn backup_updated(&self, status: &BackupStatus) {
        let _ = self.app.emit("backup:updated", status);
    }
    fn notify(&self, title: &str, body: &str) {
        let now = self.notices().add(title, body, Instant::now(), local_minute());
        if let Some((title, body)) = now {
            self.show(&title, &body);
        }
    }
    fn open(&self, target: &str) -> std::result::Result<(), String> {
        let opener = self.app.opener();
        let done = if target.starts_with("https://") {
            opener.open_url(target, None::<&str>)
        } else {
            // As a click on a path in a report opens it: one that could run a program is shown in its folder instead.
            let path = std::path::Path::new(target);
            if commands::opens_directly(path, path.is_dir()) {
                opener.open_path(target, None::<&str>)
            } else {
                opener.reveal_item_in_dir(path)
            }
        };
        done.map_err(|e| e.to_string())
    }
    fn settings_changed(&self, s: &AppSettings) {
        if let Some(item) = self.pause_item.get() {
            let _ = item.set_text(pause_label(s.paused));
        }
        self.notices().set(s.notifications.clone());
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &tauri::App, office: Office, sink: &TauriSink) -> tauri::Result<()> {
    let paused = office.settings().map(|s| s.paused).unwrap_or(false);
    let open = MenuItem::with_id(app, "open", "Open Omega Star", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", pause_label(paused), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &quit])?;
    let _ = sink.pause_item.set(pause);
    let mut tray = TrayIconBuilder::with_id("main").tooltip("Omega Star").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.on_menu_event(move |app, ev| match ev.id.as_ref() {
        "open" => show_main(app),
        "pause" => {
            if let Ok(s) = office.settings() {
                let _ = office.set_paused(!s.paused);
            }
        }
        "quit" => {
            app.exit(0);
        }
        _ => {}
    })
    .on_tray_icon_event(|tray, ev| {
        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
            show_main(tray.app_handle());
        }
    })
    .build(app)?;
    Ok(())
}

/// The standard menu, with Cmd+Q closing the window instead of quitting: schedules only run while
/// the app does. Quit stays in the tray and the Dock.
#[cfg(target_os = "macos")]
fn app_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    use tauri::menu::MenuItemKind;
    let menu = Menu::default(app)?;
    let Some(MenuItemKind::Submenu(app_submenu)) = menu.items()?.into_iter().next() else {
        log!("could not find the app menu to replace Quit");
        return Ok(menu);
    };
    let quit = app_submenu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Predefined(p) if p.text().is_ok_and(|t| t.starts_with("Quit")) => Some(p),
        _ => None,
    });
    let Some(quit) = quit else {
        log!("could not find Quit in the app menu");
        return Ok(menu);
    };
    app_submenu.remove(&quit)?;
    app_submenu.append(&MenuItem::with_id(app, "keep-running", "Close and Keep Running", true, Some("CmdOrCtrl+Q"))?)?;
    Ok(menu)
}

fn autostart() -> tauri::plugin::TauriPlugin<Wry> {
    let b = tauri_plugin_autostart::Builder::new().args(["--background"]);
    #[cfg(target_os = "macos")]
    let b = b.macos_launcher(tauri_plugin_autostart::MacosLauncher::LaunchAgent);
    b.build()
}

/// Keys for the login item: launchd starts Omega Star again when the copy it started dies of a crash
/// signal (a panic aborts), and only then, at most once a minute. A Quit, a force quit or a start that
/// could not open the office stays down, and a restarted copy meets the single-instance check like any
/// other, so it never runs beside one you opened.
#[cfg(target_os = "macos")]
const KEEP_ALIVE: &str =
    "<key>KeepAlive</key>\n  <dict>\n    <key>Crashed</key>\n    <true/>\n  </dict>\n  <key>ThrottleInterval</key>\n  <integer>60</integer>\n  ";

/// The autostart plugin's plist with KEEP_ALIVE added at the end of its dict, or `None` when it has a
/// KeepAlive already or is not a login item's plist. Pure.
#[cfg(target_os = "macos")]
fn with_keep_alive(plist: &str) -> Option<String> {
    if plist.contains("<key>KeepAlive</key>") || !plist.contains("<key>ProgramArguments</key>") {
        return None;
    }
    let end = plist.rfind("</dict>")?;
    Some(format!("{}{KEEP_ALIVE}{}", &plist[..end], &plist[end..]))
}

/// Has the login item, when there is one, bring Omega Star back after a crash. launchd reads it at
/// login, so this takes effect from the next one.
#[cfg(target_os = "macos")]
fn keep_alive_after_crash(app: &AppHandle) {
    let Some(home) = dirs::home_dir() else { return };
    let plist = home.join("Library/LaunchAgents").join(format!("{}.plist", app.package_info().name));
    let Some(text) = std::fs::read_to_string(&plist).ok().and_then(|t| with_keep_alive(&t)) else { return };
    if let Err(e) = std::fs::write(&plist, text) {
        log!("could not have the login item restart Omega Star after a crash: {e}");
    }
}

/// A page an employee showed, for its sealed frame: `omega-visual://localhost/<run id>%2F<page id>?theme=dark`
/// (`http://omega-visual.localhost/...` on Windows).
fn visual_page(app: &AppHandle, req: &tauri::http::Request<Vec<u8>>) -> tauri::http::Response<Vec<u8>> {
    let path = req.uri().path().trim_start_matches('/').replace("%2F", "/").replace("%2f", "/");
    let dark = req.uri().query().is_some_and(|q| q.split('&').any(|kv| kv == "theme=dark"));
    let page = match (path.split_once('/'), app.try_state::<Office>()) {
        (Some((run_id, id)), Some(office)) => office.visual_page(run_id, id, dark),
        _ => Err(model::AppError::not_found("No such page.")),
    };
    let res = tauri::http::Response::builder().header("Content-Security-Policy", office::PAGE_CSP);
    match page {
        Ok(body) => res.header("Content-Type", "text/html; charset=utf-8").body(body),
        Err(e) => res.status(404).header("Content-Type", "text/plain; charset=utf-8").body(e.message.into_bytes()),
    }
    .unwrap_or_default()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    log::catch_crashes();
    let background = std::env::args().any(|a| a == "--background");
    let builder = tauri::Builder::default();
    #[cfg(target_os = "macos")]
    let builder = builder.menu(app_menu).on_menu_event(|app, ev| {
        if ev.id() == "keep-running" {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.hide();
            }
        }
    });
    builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            // A login-item relaunch while already running must not pop the window.
            if !args.iter().any(|a| a == "--background") {
                show_main(app);
            }
        }))
        .plugin(autostart())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .register_uri_scheme_protocol("omega-visual", |ctx, req| visual_page(ctx.app_handle(), &req))
        .setup(move |app| {
            let data_dir = office::default_data_dir();
            if let Err(e) = std::fs::create_dir_all(&data_dir).and_then(|_| log::to_file(&data_dir.join("omega.log"))) {
                log!("could not open omega.log: {e}");
            }
            // `package_info` carries a version set with `tauri build --config`, as dogfood builds do.
            let build = Build::new(app.package_info().version.to_string());
            log!("Omega Star {} started ({} build {}) on {}", build.version, format!("{:?}", build.channel).to_lowercase(), build.commit, data_dir.display());
            let sink = Arc::new(TauriSink { app: app.handle().clone(), pause_item: OnceLock::new(), notices: Mutex::new(Notices::default()) });
            let office = match tauri::async_runtime::block_on(Office::open(data_dir.clone(), sink.clone(), build)) {
                Ok(office) => office,
                Err(e) => {
                    // Without this the app would vanish: there is no window yet and release builds abort.
                    log!("could not open the office at {}: {e}", data_dir.display());
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("Omega Star could not open its office folder at {}.\n\n{}", data_dir.display(), e.message))
                        .title("Omega Star could not start")
                        .kind(MessageDialogKind::Error)
                        .show(move |_| handle.exit(1));
                    return Ok(());
                }
            };
            match office.settings() {
                Ok(s) => sink.settings_changed(&s),
                Err(e) => log!("could not read the notification settings: {e}"),
            }
            spawn_notices(sink.clone());
            office.spawn_scheduler();
            office.spawn_usage_monitor();
            office.spawn_account_monitor();
            office.spawn_backup_monitor();
            office.spawn_tidy_monitor();
            office.spawn_watch_monitor();
            office.spawn_transcript_sweep();
            office.spawn_transcript_compaction();
            // A login item set up by an older release does not restart after a crash yet. Other builds leave
            // the release's login item alone.
            #[cfg(target_os = "macos")]
            if model::BuildChannel::current() == model::BuildChannel::Release {
                keep_alive_after_crash(app.handle());
            }
            build_tray(app, office.clone(), &sink)?;
            app.manage(office);
            if !background {
                show_main(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::setup_status,
            commands::complete_setup,
            commands::update_company,
            commands::set_background,
            commands::org,
            commands::skill_catalog,
            commands::skill_sources,
            commands::hire,
            commands::update_employee,
            commands::let_go,
            commands::schedules,
            commands::save_schedule,
            commands::delete_schedule,
            commands::cron_preview,
            commands::run_now,
            commands::cancel_run,
            commands::start_task,
            commands::open_attachment,
            commands::read_image,
            commands::open_image,
            commands::open_report_path,
            commands::runs,
            commands::search_runs,
            commands::run_usage,
            commands::run_detail,
            commands::archived,
            commands::conversations,
            commands::thread,
            commands::set_archived,
            commands::unarchive_runs,
            commands::retry_run,
            commands::reply_to_run,
            commands::approve_blocked,
            commands::answer_card,
            commands::card_file,
            commands::parked_runs,
            commands::open_asks,
            commands::week_reviews,
            commands::employee_skills,
            commands::skill_health,
            commands::tidy,
            commands::approve_tidy,
            commands::tidy_runs,
            commands::checks,
            commands::drop_check,
            commands::read_skill_file,
            commands::save_skill_file,
            commands::company_page,
            commands::save_company_page,
            commands::skill_history,
            commands::revert_revision,
            commands::promote_skill,
            commands::import_skills,
            commands::staged_imports,
            commands::organize,
            commands::revert_run,
            commands::import_originals,
            commands::remove_originals,
            commands::settle_originals,
            commands::allow_rule,
            commands::add_person,
            commands::update_person,
            commands::remove_person,
            commands::requests,
            commands::resolve_request,
            commands::watch_request,
            commands::proposals,
            commands::approve_proposal,
            commands::decline_proposal,
            commands::settings,
            commands::save_settings,
            commands::open_data_dir,
            commands::backup_status,
            commands::backup_now,
            commands::pick_backup_dir,
            commands::open_backup_dir,
            commands::usage_limits,
            commands::refresh_usage,
            commands::use_signed_in_account,
            commands::check_update,
            commands::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Omega Star")
        .run(|app, event| {
            // Children would otherwise outlive the Office on any exit path (tray Quit, Cmd+Q, logout).
            if let tauri::RunEvent::Exit = event {
                if let Some(office) = app.try_state::<Office>() {
                    office.shutdown();
                }
            }
        });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// The plist the autostart plugin writes.
    const PLUGIN_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n  <dict>\n  <key>Label</key>\n  <string>Acme Office</string>\n  \
<key>ProgramArguments</key>\n  <array><string>/Applications/Acme Office.app/Contents/MacOS/acme-office</string><string>--background</string></array>\n  \
<key>RunAtLoad</key>\n  <true/>\n  </dict>\n</plist>";

    #[test]
    fn the_login_item_restarts_only_after_a_crash() {
        let text = with_keep_alive(PLUGIN_PLIST).unwrap();
        assert_eq!(with_keep_alive(&text), None, "added once");
        assert_eq!(with_keep_alive("<plist><dict><key>Label</key></dict></plist>"), None, "only a login item's plist");

        let dir = std::env::temp_dir().join(format!("omegastar-plist-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Acme Office.plist");
        std::fs::write(&path, &text).unwrap();
        let out = std::process::Command::new("/usr/bin/plutil").args(["-convert", "json", "-o", "-"]).arg(&path).output().unwrap();
        assert!(out.status.success(), "launchd can read it: {}", String::from_utf8_lossy(&out.stderr));
        let plist: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(plist["KeepAlive"], serde_json::json!({ "Crashed": true }));
        assert_eq!(plist["ThrottleInterval"], 60);
        assert_eq!(plist["RunAtLoad"], true);
        assert_eq!(plist["ProgramArguments"], serde_json::json!(["/Applications/Acme Office.app/Contents/MacOS/acme-office", "--background"]));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

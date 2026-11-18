mod backup;
mod cards;
mod catalog;
mod claude;
mod commands;
mod error;
#[cfg(test)]
mod fixtures;
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
mod remote;
mod sandbox;
mod schedule;
mod store;
mod threads;
mod tidy;
mod update;
mod vibe_typer;
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

/// The product's name up to 0.6.5.
pub(crate) const OLD_PRODUCT_NAME: &str = "Omega Star";

struct TauriSink {
    app: AppHandle,
    /// The phone link, which mirrors changes and notifications to a paired phone.
    remote: remote::Remote,
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
        self.remote.push(title);
    }
}

/// Minutes since midnight on this computer's clock, which quiet hours are set in.
fn local_minute() -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

/// Tells the interface each change of the phone link (`remote:updated`).
fn spawn_remote_status(app: AppHandle, remote: &remote::Remote) {
    let mut status = remote.subscribe();
    tauri::async_runtime::spawn(async move {
        while status.changed().await.is_ok() {
            let now = status.borrow_and_update().clone();
            let _ = app.emit("remote:updated", now);
        }
    });
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
        self.remote.changed(remote::What::Team);
        self.remote.changed(remote::What::Inbox);
    }
    fn run_updated(&self, run: &Run) {
        let _ = self.app.emit("run:updated", run);
        self.remote.changed(remote::What::Inbox);
    }
    fn run_events(&self, run_id: &str, events: &[RunEvent]) {
        let _ = self.app.emit("run:event", serde_json::json!({ "runId": run_id, "events": events }));
    }
    fn request_updated(&self, request: &HumanRequest) {
        let _ = self.app.emit("request:updated", request);
    }
    fn proposal_updated(&self, proposal: &HireProposal) {
        let _ = self.app.emit("proposal:updated", proposal);
        self.remote.changed(remote::What::Proposals);
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
        let done = if target.starts_with("https://") {
            self.app.opener().open_url(target, None::<&str>)
        } else {
            // As a click on a path in a report opens it.
            commands::open_or_reveal(&self.app, std::path::Path::new(target))
        };
        done.map_err(|e| e.to_string())
    }
    fn show_file(&self, root_id: &str, path: &str) -> std::result::Result<(), String> {
        self.app.emit("reader:open", serde_json::json!({ "rootId": root_id, "path": path })).map_err(|e| e.to_string())
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
    let open = MenuItem::with_id(app, "open", "Open OmegaStar", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", pause_label(paused), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &pause, &quit])?;
    let _ = sink.pause_item.set(pause);
    let mut tray = TrayIconBuilder::with_id("main").tooltip("OmegaStar").menu(&menu).show_menu_on_left_click(false);
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

/// Where an older release wrote the login item under the old name, in `home`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn old_login_item(home: &std::path::Path) -> std::path::PathBuf {
    if cfg!(target_os = "macos") {
        home.join("Library/LaunchAgents").join(format!("{OLD_PRODUCT_NAME}.plist"))
    } else {
        home.join(".config/autostart").join(format!("{OLD_PRODUCT_NAME}.desktop"))
    }
}

/// Moves a login item an older release wrote under the old name to this release's name: `enable` writes the new
/// one, then the old file goes. An item the desktop's own settings turned off is removed and stays off. Returns
/// whether there was one to move.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn carry_login_item(home: &std::path::Path, enable: impl FnOnce() -> Result<(), String>) -> Result<bool, String> {
    let old = old_login_item(home);
    if !old.exists() {
        return Ok(false);
    }
    let off = std::fs::read_to_string(&old).is_ok_and(|text| {
        let mut keys = text.lines().filter_map(|l| l.split_once('=')).map(|(key, value)| (key.trim(), value.trim()));
        keys.any(|line| matches!(line, ("Hidden", "true" | "1") | ("X-GNOME-Autostart-enabled", "false" | "0")))
    });
    if !off {
        enable()?;
    }
    std::fs::remove_file(&old).map_err(|e| e.to_string())?;
    Ok(true)
}

/// The bundle the app runs from and the name it moves to, when `exe` is the binary of a bundle named exactly
/// `Omega Star.app`.
#[cfg(target_os = "macos")]
fn renamed_bundle(exe: &std::path::Path) -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let contents = exe.parent().filter(|p| p.ends_with("MacOS"))?.parent().filter(|p| p.ends_with("Contents"))?;
    let bundle = contents.parent().filter(|p| p.ends_with(format!("{OLD_PRODUCT_NAME}.app")))?;
    Some((bundle.to_path_buf(), bundle.with_file_name("OmegaStar.app")))
}

/// Renames a bundle the updater left under the old name and opens it again from there, so Finder, the Dock and
/// Spotlight say OmegaStar. `open` starts it outside this process, since launchd ends a login item's children
/// with it. A start that finds another start renamed the bundle first leaves the opening to that one.
#[cfg(target_os = "macos")]
fn reopen_renamed() {
    let Some((old, new)) = std::env::current_exe().ok().as_deref().and_then(renamed_bundle) else { return };
    let renamed = !new.exists() && std::fs::rename(&old, &new).inspect_err(|e| log!("could not rename the app to OmegaStar: {e}")).is_ok();
    if old.exists() {
        return;
    }
    if !renamed {
        std::process::exit(0);
    }
    let open = std::process::Command::new("/usr/bin/open").arg("-n").arg(&new).arg("--args").args(std::env::args_os().skip(1)).status();
    if open.is_ok_and(|s| s.success()) {
        std::process::exit(0);
    }
    log!("could not open OmegaStar after renaming it, so it runs on under its old name");
    let _ = std::fs::rename(&new, &old);
}

/// Keys for the login item: launchd starts OmegaStar again when the copy it started dies of a crash
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

/// Has the login item, when there is one, bring OmegaStar back after a crash. launchd reads it at
/// login, so this takes effect from the next one.
#[cfg(target_os = "macos")]
fn keep_alive_after_crash(app: &AppHandle) {
    let Some(home) = dirs::home_dir() else { return };
    let plist = home.join("Library/LaunchAgents").join(format!("{}.plist", app.package_info().name));
    let Some(text) = std::fs::read_to_string(&plist).ok().and_then(|t| with_keep_alive(&t)) else { return };
    if let Err(e) = std::fs::write(&plist, text) {
        log!("could not have the login item restart OmegaStar after a crash: {e}");
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
    #[cfg(target_os = "macos")]
    reopen_renamed();
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
            // A test build pointed at the release office writes nothing there, not even its log: `Office::open`
            // refuses it with the same check, and the dialog below says why.
            let refused = sandbox::check_office(model::BuildChannel::current(), &data_dir, sandbox::release_dir().as_deref()).is_err();
            if !refused {
                if let Err(e) = std::fs::create_dir_all(&data_dir).and_then(|_| log::to_file(&data_dir.join("omega.log"))) {
                    log!("could not open omega.log: {e}");
                }
            }
            // `package_info` carries a version set with `tauri build --config`, as dogfood builds do.
            let build = Build::new(app.package_info().version.to_string());
            log!("OmegaStar {} started ({} build {}) on {}", build.version, format!("{:?}", build.channel).to_lowercase(), build.commit, data_dir.display());
            let remote = remote::Remote::new(remote::relay());
            app.manage(remote.clone());
            let sink = Arc::new(TauriSink { app: app.handle().clone(), remote: remote.clone(), pause_item: OnceLock::new(), notices: Mutex::new(Notices::default()) });
            let office = match tauri::async_runtime::block_on(Office::open(data_dir.clone(), sink.clone(), build)) {
                Ok(office) => office,
                Err(e) => {
                    // Without this the app would vanish: there is no window yet and release builds abort.
                    log!("could not open the office at {}: {e}", data_dir.display());
                    let handle = app.handle().clone();
                    app.dialog()
                        .message(format!("OmegaStar could not open its office folder at {}.\n\n{}", data_dir.display(), e.message))
                        .title("OmegaStar could not start")
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
            spawn_remote_status(app.handle().clone(), &remote);
            remote.start(office.clone());
            // Other builds leave the release's login item alone.
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            if model::BuildChannel::current() == model::BuildChannel::Release {
                if let Some(home) = dirs::home_dir() {
                    let enable = || tauri_plugin_autostart::ManagerExt::autolaunch(app.handle()).enable().map_err(|e| e.to_string());
                    match carry_login_item(&home, enable) {
                        Ok(true) => log!("moved the login item off the name Omega Star"),
                        Ok(false) => {}
                        Err(e) => log!("could not move the login item to the name OmegaStar: {e}"),
                    }
                }
                #[cfg(target_os = "macos")]
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
            commands::models,
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
            commands::report_file,
            commands::runs,
            commands::search_runs,
            commands::run_usage,
            commands::team_stats,
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
            commands::last_dictation,
            commands::check_update,
            commands::install_update,
            commands::remote_status,
            commands::remote_pair_start,
            commands::remote_pair_cancel,
            commands::remote_unpair,
            commands::remote_set_push,
        ])
        .build(tauri::generate_context!())
        .expect("error while building OmegaStar")
        .run(|app, event| {
            // Children would otherwise outlive the Office on any exit path (tray Quit, Cmd+Q, logout).
            if let tauri::RunEvent::Exit = event {
                if let Some(office) = app.try_state::<Office>() {
                    office.shutdown();
                }
            }
        });
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests {
    use super::*;

    /// The plist the autostart plugin writes.
    #[cfg(target_os = "macos")]
    const PLUGIN_PLIST: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
\"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n  <dict>\n  <key>Label</key>\n  <string>Acme Office</string>\n  \
<key>ProgramArguments</key>\n  <array><string>/Applications/Acme Office.app/Contents/MacOS/acme-office</string><string>--background</string></array>\n  \
<key>RunAtLoad</key>\n  <true/>\n  </dict>\n</plist>";

    #[test]
    #[cfg(target_os = "macos")]
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

    #[test]
    fn a_login_item_under_the_old_name_moves_to_the_new_one() {
        let home = std::env::temp_dir().join(format!("omegastar-home-{}", uuid::Uuid::new_v4()));
        let old = old_login_item(&home);
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        let text = if cfg!(target_os = "macos") {
            "<plist version=\"1.0\">\n  <dict>\n  <key>Label</key>\n  <string>Omega Star</string>\n  <key>ProgramArguments</key>\n  \
<array><string>/Applications/Omega Star.app/Contents/MacOS/omega-star</string><string>--background</string></array>\n  </dict>\n</plist>"
        } else {
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Omega Star\nExec=/home/alex/Apps/Omega.Star_0.6.5_amd64.AppImage --background\n"
        };
        std::fs::write(&old, text).unwrap();
        let new = old.with_file_name("OmegaStar");
        assert_eq!(carry_login_item(&home, || std::fs::write(&new, "new").map_err(|e| e.to_string())), Ok(true));
        assert!(!old.exists() && new.exists());
        assert_eq!(carry_login_item(&home, || panic!("nothing to move")), Ok(false), "a second start finds nothing");

        std::fs::write(&old, "old").unwrap();
        assert_eq!(carry_login_item(&home, || Err("no app path".into())), Err("no app path".into()));
        assert!(old.exists(), "the old one stays when the new one could not be written");

        for off in ["X-GNOME-Autostart-enabled=false", "X-GNOME-Autostart-enabled = 0", "Hidden = true"] {
            std::fs::write(&old, format!("{text}\n{off}\n")).unwrap();
            assert_eq!(carry_login_item(&home, || panic!("it was turned off")), Ok(true), "{off}");
            assert!(!old.exists(), "an item the desktop turned off goes and no new one is written");
        }
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn only_a_bundle_named_omega_star_is_renamed() {
        let pair = renamed_bundle(std::path::Path::new("/Applications/Omega Star.app/Contents/MacOS/omega-star"));
        assert_eq!(pair, Some(("/Applications/Omega Star.app".into(), "/Applications/OmegaStar.app".into())));
        for exe in [
            "/Applications/OmegaStar.app/Contents/MacOS/omega-star",
            "/Applications/Omega Star Dogfood.app/Contents/MacOS/omega-star",
            "/Users/alex/code/omegastar/src-tauri/target/release/omega-star",
            "/Volumes/Omega Star.app/Helpers/Contents/MacOS/omega-star",
        ] {
            assert_eq!(renamed_bundle(std::path::Path::new(exe)), None, "{exe}");
        }
    }
}

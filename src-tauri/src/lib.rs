mod backup;
mod catalog;
mod claude;
mod commands;
mod error;
mod health;
mod log;
mod mcp;
mod model;
mod office;
mod org;
mod own_skills;
mod reader;
mod schedule;
mod store;
mod threads;
mod tidy;
mod update;
mod watch;

use log::log;
use model::{AppSettings, BackupStatus, Build, HireProposal, HumanRequest, Run, RunEvent, UsageLimits};
use office::{EventSink, Office};
use std::sync::{Arc, OnceLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, WindowEvent, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;

struct TauriSink {
    app: AppHandle,
    pause_item: OnceLock<MenuItem<Wry>>,
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
        if let Err(e) = self.app.notification().builder().title(title).body(body).show() {
            log!("notification failed: {e}");
        }
    }
    fn settings_changed(&self, s: &AppSettings) {
        if let Some(item) = self.pause_item.get() {
            let _ = item.set_text(pause_label(s.paused));
        }
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
        .setup(move |app| {
            let data_dir = office::default_data_dir();
            if let Err(e) = std::fs::create_dir_all(&data_dir).and_then(|_| log::to_file(&data_dir.join("omega.log"))) {
                log!("could not open omega.log: {e}");
            }
            // `package_info` carries a version set with `tauri build --config`, as dogfood builds do.
            let build = Build::new(app.package_info().version.to_string());
            log!("Omega Star {} started ({} build {})", build.version, format!("{:?}", build.channel).to_lowercase(), build.commit);
            let sink = Arc::new(TauriSink { app: app.handle().clone(), pause_item: OnceLock::new() });
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
            office.spawn_scheduler();
            office.spawn_usage_monitor();
            office.spawn_account_monitor();
            office.spawn_backup_monitor();
            office.spawn_tidy_monitor();
            office.spawn_watch_monitor();
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
            commands::send_chat,
            commands::start_task,
            commands::open_attachment,
            commands::read_image,
            commands::open_image,
            commands::open_report_path,
            commands::new_chat,
            commands::chat,
            commands::runs,
            commands::run_detail,
            commands::archived,
            commands::conversations,
            commands::thread,
            commands::set_archived,
            commands::reply_to_run,
            commands::approve_blocked,
            commands::open_asks,
            commands::employee_skills,
            commands::skill_health,
            commands::tidy,
            commands::approve_tidy,
            commands::tidy_runs,
            commands::checks,
            commands::drop_check,
            commands::read_skill_file,
            commands::save_skill_file,
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

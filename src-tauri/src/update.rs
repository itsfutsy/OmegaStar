use crate::error::{AppError, Result};
use crate::office::Office;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

async fn available(app: &AppHandle) -> Result<Option<Update>> {
    if cfg!(debug_assertions) {
        return Ok(None);
    }
    let failed = |e: tauri_plugin_updater::Error| AppError::internal(format!("Could not check for updates: {e}"));
    let handle = app.clone();
    // Windows installs exit the process directly, past the RunEvent::Exit handler in lib.rs.
    let updater = app.updater_builder().on_before_exit(move || {
        if let Some(office) = handle.try_state::<Office>() {
            office.shutdown();
        }
        handle.cleanup_before_exit();
    });
    updater.build().map_err(failed)?.check().await.map_err(failed)
}

pub async fn check(app: &AppHandle) -> Result<Option<String>> {
    Ok(available(app).await?.map(|u| u.version.to_string()))
}

pub async fn install(app: &AppHandle) -> Result<()> {
    let update = available(app).await?.ok_or_else(|| AppError::conflict("There is no update to install."))?;
    let bytes = update.download(|_, _| {}, || {}).await.map_err(|e| AppError::internal(format!("Could not download the update: {e}")))?;
    update.install(bytes).map_err(|e| AppError::internal(format!("Could not install the update: {e}")))?;
    app.request_restart();
    Ok(())
}

mod db;
mod error;
mod models;
mod scan;

use std::sync::Mutex;

use tauri::{Manager, State};

use crate::db::{Database, app_data_dir};
use crate::error::Error;
use crate::models::{Clip, Folder, ScanSummary};
use crate::scan::{nested_conflict, normalize_folder, scan_all};

struct AppState {
    db: Mutex<Database>,
}

#[tauri::command]
fn list_folders(state: State<AppState>) -> Result<Vec<Folder>, Error> {
    let db = lock_db(&state)?;
    db.list_folders()
}

#[tauri::command]
fn add_folder(state: State<AppState>, path: String) -> Result<Folder, Error> {
    let canonical = normalize_folder(std::path::Path::new(&path))?;
    let db = lock_db(&state)?;
    let existing = db.list_folders()?;
    if let Some(conflict) = nested_conflict(&existing, &canonical) {
        return Err(Error::message(format!(
            "That folder overlaps {conflict}. Watch one of them, not both."
        )));
    }
    db.add_folder(&canonical.to_string_lossy())
}

#[tauri::command]
fn remove_folder(state: State<AppState>, id: i64) -> Result<(), Error> {
    let db = lock_db(&state)?;
    db.remove_folder(id)
}

#[tauri::command]
fn scan_folders(state: State<AppState>) -> Result<ScanSummary, Error> {
    let db = lock_db(&state)?;
    scan_all(&db)
}

#[tauri::command]
fn list_clips(state: State<AppState>) -> Result<Vec<Clip>, Error> {
    let db = lock_db(&state)?;
    db.list_clips()
}

#[tauri::command]
fn set_viewed(state: State<AppState>, id: i64, viewed: bool) -> Result<Clip, Error> {
    let db = lock_db(&state)?;
    db.set_viewed(id, viewed)
}

fn lock_db<'a>(
    state: &'a State<'a, AppState>,
) -> Result<std::sync::MutexGuard<'a, Database>, Error> {
    state
        .db
        .lock()
        .map_err(|_| Error::message("Database is unavailable."))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let dir = app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let db = Database::open(&dir.join("ragetrim.db"))?;
            app.manage(AppState { db: Mutex::new(db) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_folders,
            add_folder,
            remove_folder,
            scan_folders,
            list_clips,
            set_viewed
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

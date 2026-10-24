mod db;
mod discord;
mod error;
mod ffmpeg;
mod models;
mod scan;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, State};

use crate::db::{Database, app_data_dir};
use crate::error::Error;
use crate::ffmpeg::{
    ExportMode, export_args, export_file_name, find_tool, is_ffmpeg_binary, preview_args,
    probe_file, run_tool, validate_range,
};
use crate::models::{AppSettings, Clip, ExportResult, Folder, ScanSummary};
use crate::scan::{nested_conflict, normalize_folder, scan_all};

struct AppState {
    db: Mutex<Database>,
    busy: Arc<AtomicBool>,
}

struct BusyGuard(Arc<AtomicBool>);

impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

#[tauri::command]
fn list_folders(state: State<AppState>) -> Result<Vec<Folder>, Error> {
    let db = lock_db(&state)?;
    db.list_folders()
}

#[tauri::command]
fn add_folder(app: AppHandle, state: State<AppState>, path: String) -> Result<Folder, Error> {
    let canonical = normalize_folder(Path::new(&path))?;
    allow_asset_dir(&app, &canonical)?;
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

#[tauri::command]
fn set_edited(state: State<AppState>, id: i64, edited: bool) -> Result<Clip, Error> {
    let db = lock_db(&state)?;
    db.set_edited(id, edited)
}

#[tauri::command]
fn set_posted(state: State<AppState>, id: i64, posted: bool) -> Result<Clip, Error> {
    let db = lock_db(&state)?;
    db.set_posted(id, posted)
}

#[tauri::command]
fn discord_status() -> Result<discord::DiscordStatus, Error> {
    discord::status()
}

#[tauri::command]
fn save_discord_webhook(url: String) -> Result<discord::DiscordStatus, Error> {
    discord::save_webhook(&url)?;
    discord::status()
}

#[tauri::command]
fn clear_discord_webhook() -> Result<discord::DiscordStatus, Error> {
    discord::clear_webhook()?;
    discord::status()
}

#[tauri::command]
fn test_discord_webhook() -> Result<discord::WebhookCheck, Error> {
    discord::test_webhook()
}

#[tauri::command]
fn post_clip(state: State<AppState>, id: i64, message: String) -> Result<Clip, Error> {
    let _job = begin_job(&state)?;
    let (path, file_name) = {
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        if clip.missing {
            return Err(Error::message("That clip is missing."));
        }
        let Some((path, _, _)) = db.latest_export(id)? else {
            return Err(Error::message("Export a trim before posting."));
        };
        let export_dir = db.export_dir()?;
        let path = PathBuf::from(path);
        if !export_is_inside(&export_dir, &path) {
            return Err(Error::message("That export is outside the export folder."));
        }
        if !path.is_file() {
            return Err(Error::message("That export is missing. Trim it again."));
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&clip.file_name)
            .to_string();
        (path, file_name)
    };
    discord::post_file(&path, &file_name, &message)?;
    let db = lock_db(&state)?;
    db.set_posted(id, true)
}

fn export_is_inside(root: &Path, candidate: &Path) -> bool {
    let Ok(root) = dunce::canonicalize(root) else {
        return false;
    };
    let Ok(candidate) = dunce::canonicalize(candidate) else {
        return false;
    };
    candidate.starts_with(root)
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<AppSettings, Error> {
    let db = lock_db(&state)?;
    settings_from(&db)
}

#[tauri::command]
fn set_export_dir(
    app: AppHandle,
    state: State<AppState>,
    path: String,
) -> Result<AppSettings, Error> {
    let canonical = normalize_folder(Path::new(&path))?;
    std::fs::create_dir_all(&canonical)?;
    allow_asset_dir(&app, &canonical)?;
    let db = lock_db(&state)?;
    db.set_setting("export_dir", &canonical.to_string_lossy())?;
    settings_from(&db)
}

#[tauri::command]
fn set_compress_target(state: State<AppState>, bytes: u64) -> Result<AppSettings, Error> {
    const MIN: u64 = 1024 * 1024;
    const MAX: u64 = 100 * 1024 * 1024;
    if !(MIN..=MAX).contains(&bytes) {
        return Err(Error::message(
            "Compress target must be between 1 MB and 100 MB.",
        ));
    }
    let db = lock_db(&state)?;
    db.set_setting("compress_target_bytes", &bytes.to_string())?;
    settings_from(&db)
}

#[tauri::command]
fn set_ffmpeg_path(state: State<AppState>, path: String) -> Result<AppSettings, Error> {
    let path = PathBuf::from(path);
    if !path.is_file() || !is_ffmpeg_binary(&path) {
        return Err(Error::message("Pick ffmpeg.exe."));
    }
    let canonical = dunce::canonicalize(&path).unwrap_or(path);
    let db = lock_db(&state)?;
    db.set_setting("ffmpeg_path", &canonical.to_string_lossy())?;
    settings_from(&db)
}

#[tauri::command]
fn probe_clip(state: State<AppState>, id: i64) -> Result<Clip, Error> {
    let _job = begin_job(&state)?;
    let (input, ffprobe) = {
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        if clip.missing {
            return Err(Error::message("That clip is missing."));
        }
        let ffprobe = required_tool(&db, "ffprobe")?;
        (PathBuf::from(clip.path), ffprobe)
    };
    assert_safe_path(&input)?;
    let probe = probe_file(&ffprobe, &input)?;
    let db = lock_db(&state)?;
    db.save_probe(
        id,
        probe.duration_ms,
        probe.width,
        probe.height,
        probe.video_codec.as_deref(),
    )
}

#[tauri::command]
fn make_preview(app: AppHandle, state: State<AppState>, id: i64) -> Result<String, Error> {
    let _job = begin_job(&state)?;
    let (input, ffmpeg, output, duration_ms) = {
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        if clip.missing {
            return Err(Error::message("That clip is missing."));
        }
        let ffmpeg = required_tool(&db, "ffmpeg")?;
        let dir = app_data_dir()?.join("proxies");
        std::fs::create_dir_all(&dir)?;
        let output = dir.join(format!("{}-{}.mp4", clip.id, clip.modified_ms));
        (
            PathBuf::from(clip.path),
            ffmpeg,
            output,
            clip.duration_ms.unwrap_or(0),
        )
    };
    if output.is_file()
        && std::fs::metadata(&output)
            .map(|meta| meta.len())
            .unwrap_or(0)
            > 0
    {
        return Ok(output.to_string_lossy().into_owned());
    }
    assert_safe_path(&input)?;
    let args = preview_args(&input, &output);
    if let Err(err) = run_tool(&app, &ffmpeg, &args, id, "preview", duration_ms) {
        let _ = std::fs::remove_file(&output);
        return Err(err);
    }
    Ok(output.to_string_lossy().into_owned())
}

#[tauri::command]
fn export_clip(
    app: AppHandle,
    state: State<AppState>,
    id: i64,
    in_ms: i64,
    out_ms: i64,
    mode: String,
) -> Result<ExportResult, Error> {
    let _job = begin_job(&state)?;
    let mode = ExportMode::parse(&mode)?;
    let (input, ffmpeg, output, target, height, duration_ms) = {
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        if clip.missing {
            return Err(Error::message("That clip is missing."));
        }
        validate_range(in_ms, out_ms, clip.duration_ms)?;
        let ffmpeg = required_tool(&db, "ffmpeg")?;
        let export_dir = db.export_dir()?;
        std::fs::create_dir_all(&export_dir)?;
        allow_asset_dir(&app, &export_dir)?;
        let file_name = export_file_name(&clip.file_name, in_ms, out_ms, mode);
        let output = unique_output(&export_dir, &file_name);
        (
            PathBuf::from(clip.path),
            ffmpeg,
            output,
            db.compress_target_bytes()?,
            clip.height,
            clip.duration_ms.unwrap_or(out_ms - in_ms),
        )
    };
    assert_safe_path(&input)?;
    assert_safe_path(&output)?;
    let args = export_args(&input, &output, in_ms, out_ms, mode, target, height);
    if let Err(err) = run_tool(
        &app,
        &ffmpeg,
        &args,
        id,
        "export",
        duration_ms.max(out_ms - in_ms),
    ) {
        let _ = std::fs::remove_file(&output);
        return Err(err);
    }
    let db = lock_db(&state)?;
    let clip = db.record_export(id, &output.to_string_lossy(), in_ms, out_ms, mode.as_str())?;
    Ok(ExportResult {
        path: output.to_string_lossy().into_owned(),
        clip,
    })
}

fn settings_from(db: &Database) -> Result<AppSettings, Error> {
    let configured = db.get_setting("ffmpeg_path")?.map(PathBuf::from);
    let ffmpeg = find_tool("ffmpeg", configured.as_deref());
    let ffprobe = find_tool("ffprobe", ffmpeg.as_deref());
    Ok(AppSettings {
        export_dir: db.export_dir()?.to_string_lossy().into_owned(),
        compress_target_bytes: db.compress_target_bytes()?,
        ffmpeg_path: ffmpeg.map(|path| path.to_string_lossy().into_owned()),
        ffprobe_path: ffprobe.map(|path| path.to_string_lossy().into_owned()),
    })
}

fn required_tool(db: &Database, name: &str) -> Result<PathBuf, Error> {
    let configured = db.get_setting("ffmpeg_path")?.map(PathBuf::from);
    find_tool(name, configured.as_deref()).ok_or_else(|| {
        Error::message("FFmpeg was not found. Install it or point RageTrim at ffmpeg.exe.")
    })
}

fn begin_job(state: &AppState) -> Result<BusyGuard, Error> {
    let busy = Arc::clone(&state.busy);
    if busy.swap(true, Ordering::SeqCst) {
        return Err(Error::message("Wait for the current job to finish."));
    }
    Ok(BusyGuard(busy))
}

fn allow_asset_dir(app: &AppHandle, path: &Path) -> Result<(), Error> {
    app.asset_protocol_scope()
        .allow_directory(path, true)
        .map_err(|_| Error::message("Couldn't open that folder for preview."))
}

fn assert_safe_path(path: &Path) -> Result<(), Error> {
    let text = path.to_string_lossy();
    if text.starts_with('-') || text.contains('\0') {
        return Err(Error::message("That file path can't be sent to FFmpeg."));
    }
    Ok(())
}

fn unique_output(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("clip");
    for index in 2..1000 {
        let next = dir.join(format!("{stem}-{index}.mp4"));
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!("{stem}-{}.mp4", crate::db::now_ms()))
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
            let handle = app.handle().clone();
            allow_asset_dir(&handle, &dir)?;
            for folder in db.list_folders()? {
                allow_asset_dir(&handle, Path::new(&folder.path))?;
            }
            let export_dir = db.export_dir()?;
            std::fs::create_dir_all(&export_dir)?;
            allow_asset_dir(&handle, &export_dir)?;
            app.manage(AppState {
                db: Mutex::new(db),
                busy: Arc::new(AtomicBool::new(false)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_folders,
            add_folder,
            remove_folder,
            scan_folders,
            list_clips,
            set_viewed,
            set_edited,
            set_posted,
            discord_status,
            save_discord_webhook,
            clear_discord_webhook,
            test_discord_webhook,
            post_clip,
            get_settings,
            set_export_dir,
            set_compress_target,
            set_ffmpeg_path,
            probe_clip,
            make_preview,
            export_clip
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

mod db;
mod discord;
mod error;
mod ffmpeg;
mod models;
mod scan;
mod watch;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Manager, State};

use crate::db::{Database, app_data_dir};
use crate::error::Error;
use crate::ffmpeg::{
    ExportMode, bundled_tool_dirs, export_args, export_file_name, find_tool, preview_args,
    probe_file, run_quiet, run_tool, thumbnail_args, validate_range,
};
use crate::models::{AppSettings, Clip, ExportResult, Folder, ScanSummary};
use crate::scan::{contained_file, nested_conflict, normalize_folder, paths_overlap, scan_all};

pub struct AppState {
    pub db: Mutex<Database>,
    busy: Arc<AtomicBool>,
    thumbs: Mutex<()>,
    watcher: Mutex<Option<notify::RecommendedWatcher>>,
    tool_dirs: Vec<PathBuf>,
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
    let folder = {
        let db = lock_db(&state)?;
        let existing = db.list_folders()?;
        if let Some(conflict) = nested_conflict(&existing, &canonical) {
            return Err(Error::message(format!(
                "That folder overlaps {conflict}. Watch one of them, not both."
            )));
        }
        let export_dir = db.export_dir()?;
        if paths_overlap(&export_dir, &canonical) {
            return Err(Error::message(
                "That folder overlaps the export folder. Pick a different one so exports are not indexed as clips.",
            ));
        }
        db.add_folder(&canonical.to_string_lossy())?
    };
    if let Ok(mut watcher) = state.watcher.lock() {
        if let Some(watcher) = watcher.as_mut() {
            watch::watch_folder(watcher, &canonical);
        }
    }
    Ok(folder)
}

#[tauri::command]
fn remove_folder(state: State<AppState>, id: i64) -> Result<(), Error> {
    let removed = {
        let db = lock_db(&state)?;
        let path = db
            .list_folders()?
            .into_iter()
            .find(|folder| folder.id == id)
            .map(|folder| folder.path);
        db.remove_folder(id)?;
        path
    };
    if let Some(path) = removed {
        if let Ok(mut watcher) = state.watcher.lock() {
            if let Some(watcher) = watcher.as_mut() {
                watch::unwatch_folder(watcher, Path::new(&path));
            }
        }
    }
    Ok(())
}

#[tauri::command]
fn scan_folders(state: State<AppState>) -> Result<ScanSummary, Error> {
    let db = lock_db(&state)?;
    scan_all(&db)
}

#[tauri::command]
fn ensure_thumbnail(state: State<AppState>, id: i64) -> Result<Option<String>, Error> {
    let _thumb = state
        .thumbs
        .lock()
        .map_err(|_| Error::message("Couldn't make a thumbnail."))?;
    let clip = {
        let db = lock_db(&state)?;
        db.clip_by_id(id)?
    };
    if clip.missing {
        return Ok(None);
    }
    let dir = app_data_dir()?.join("thumbs");
    std::fs::create_dir_all(&dir)?;
    let output = dir.join(format!("{}-{}.jpg", clip.id, clip.modified_ms));
    if output.is_file()
        && std::fs::metadata(&output)
            .map(|meta| meta.len())
            .unwrap_or(0)
            > 0
    {
        return Ok(Some(output.to_string_lossy().into_owned()));
    }
    let tool_dirs = state.tool_dirs.clone();
    let db = lock_db(&state)?;
    let Some(ffmpeg) = find_tool(
        "ffmpeg",
        db.get_setting("ffmpeg_path")?.as_deref().map(Path::new),
        &tool_dirs,
    ) else {
        return Ok(None);
    };
    drop(db);
    let Ok(input) = contained_file(Path::new(&clip.folder_path), Path::new(&clip.path)) else {
        return Ok(None);
    };
    if assert_safe_path(&input).is_err() {
        return Ok(None);
    }
    let first = thumbnail_args(&input, &output, "1");
    if run_quiet(&ffmpeg, &first).is_err() {
        let _ = std::fs::remove_file(&output);
        let retry = thumbnail_args(&input, &output, "0");
        if run_quiet(&ffmpeg, &retry).is_err() {
            let _ = std::fs::remove_file(&output);
            return Ok(None);
        }
    }
    Ok(Some(output.to_string_lossy().into_owned()))
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
        if !path.is_file() {
            return Err(Error::message("That export is missing. Trim it again."));
        }
        if contained_file(&export_dir, &path).is_err() {
            return Err(Error::message("That export is outside the export folder."));
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

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<AppSettings, Error> {
    let tool_dirs = state.tool_dirs.clone();
    let db = lock_db(&state)?;
    settings_from(&db, &tool_dirs)
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
    let tool_dirs = state.tool_dirs.clone();
    let db = lock_db(&state)?;
    if db
        .list_folders()?
        .iter()
        .any(|folder| paths_overlap(Path::new(&folder.path), &canonical))
    {
        return Err(Error::message(
            "That export folder overlaps a watched clip folder. Pick a different one so exports are not scanned as clips.",
        ));
    }
    db.set_setting("export_dir", &canonical.to_string_lossy())?;
    settings_from(&db, &tool_dirs)
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
    let tool_dirs = state.tool_dirs.clone();
    let db = lock_db(&state)?;
    db.set_setting("compress_target_bytes", &bytes.to_string())?;
    settings_from(&db, &tool_dirs)
}

#[tauri::command]
fn probe_clip(state: State<AppState>, id: i64) -> Result<Clip, Error> {
    let (input, ffprobe) = {
        let tool_dirs = state.tool_dirs.clone();
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        let input = clip_source(&clip)?;
        let ffprobe = required_tool(&db, "ffprobe", &tool_dirs)?;
        (input, ffprobe)
    };
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
        let tool_dirs = state.tool_dirs.clone();
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        let input = clip_source(&clip)?;
        let ffmpeg = required_tool(&db, "ffmpeg", &tool_dirs)?;
        let dir = app_data_dir()?.join("proxies");
        std::fs::create_dir_all(&dir)?;
        let output = dir.join(format!("{}-{}.mp4", clip.id, clip.modified_ms));
        (input, ffmpeg, output, clip.duration_ms.unwrap_or(0))
    };
    if output.is_file()
        && std::fs::metadata(&output)
            .map(|meta| meta.len())
            .unwrap_or(0)
            > 0
    {
        return Ok(output.to_string_lossy().into_owned());
    }
    assert_safe_path(&output)?;
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
    let (input, ffmpeg, output, target, height) = {
        let tool_dirs = state.tool_dirs.clone();
        let db = lock_db(&state)?;
        let clip = db.clip_by_id(id)?;
        let input = clip_source(&clip)?;
        validate_range(in_ms, out_ms, clip.duration_ms)?;
        let ffmpeg = required_tool(&db, "ffmpeg", &tool_dirs)?;
        let export_dir = db.export_dir()?;
        std::fs::create_dir_all(&export_dir)?;
        allow_asset_dir(&app, &export_dir)?;
        let file_name = export_file_name(&clip.file_name, in_ms, out_ms, mode);
        let output = unique_output(&export_dir, &file_name);
        (
            input,
            ffmpeg,
            output,
            db.compress_target_bytes()?,
            clip.height,
        )
    };
    assert_safe_path(&output)?;
    let args = export_args(&input, &output, in_ms, out_ms, mode, target, height);
    if let Err(err) = run_tool(
        &app,
        &ffmpeg,
        &args,
        id,
        "export",
        out_ms.saturating_sub(in_ms),
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

fn settings_from(db: &Database, tool_dirs: &[PathBuf]) -> Result<AppSettings, Error> {
    let configured = db.get_setting("ffmpeg_path")?.map(PathBuf::from);
    let ffmpeg = find_tool("ffmpeg", configured.as_deref(), tool_dirs);
    let ffprobe = find_tool("ffprobe", ffmpeg.as_deref(), tool_dirs);
    Ok(AppSettings {
        export_dir: db.export_dir()?.to_string_lossy().into_owned(),
        compress_target_bytes: db.compress_target_bytes()?,
        ffmpeg_path: ffmpeg.map(|path| path.to_string_lossy().into_owned()),
        ffprobe_path: ffprobe.map(|path| path.to_string_lossy().into_owned()),
    })
}

fn required_tool(db: &Database, name: &str, tool_dirs: &[PathBuf]) -> Result<PathBuf, Error> {
    let configured = db.get_setting("ffmpeg_path")?.map(PathBuf::from);
    find_tool(name, configured.as_deref(), tool_dirs)
        .ok_or_else(|| Error::message("video tools failed to load"))
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

fn clip_source(clip: &crate::models::Clip) -> Result<PathBuf, Error> {
    if clip.missing {
        return Err(Error::message("That clip is missing."));
    }
    let input = contained_file(Path::new(&clip.folder_path), Path::new(&clip.path))?;
    assert_safe_path(&input)?;
    Ok(input)
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
    let path = Path::new(file_name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("clip");
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("mp4");
    for index in 2..1000 {
        let next = dir.join(format!("{stem}-{index}.{ext}"));
        if !next.exists() {
            return next;
        }
    }
    dir.join(format!("{stem}-{}.{ext}", crate::db::now_ms()))
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
            let tool_dirs = bundled_tool_dirs(app.path().resource_dir().ok().as_deref());
            app.manage(AppState {
                db: Mutex::new(db),
                busy: Arc::new(AtomicBool::new(false)),
                thumbs: Mutex::new(()),
                watcher: Mutex::new(None),
                tool_dirs,
            });
            let handle = app.handle().clone();
            if let Ok(watcher) = watch::start(handle) {
                app.state::<AppState>()
                    .watcher
                    .lock()
                    .map_err(|err| std::io::Error::other(err.to_string()))?
                    .replace(watcher);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_folders,
            add_folder,
            remove_folder,
            scan_folders,
            list_clips,
            ensure_thumbnail,
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
            probe_clip,
            make_preview,
            export_clip
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

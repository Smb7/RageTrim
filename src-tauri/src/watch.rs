use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter, Manager};

use crate::error::Error;
use crate::scan::scan_all;

pub fn start(app: AppHandle) -> Result<RecommendedWatcher, Error> {
    let (tx, rx) = mpsc::channel();
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            if result.is_ok() {
                let _ = tx.send(());
            }
        },
        notify::Config::default(),
    )
    .map_err(|_| Error::message("Couldn't watch clip folders."))?;

    let folders = {
        let state = app.state::<crate::AppState>();
        let db = state
            .db
            .lock()
            .map_err(|_| Error::message("Database is unavailable."))?;
        db.list_folders()?
    };
    for folder in folders {
        let _ = watcher.watch(Path::new(&folder.path), RecursiveMode::Recursive);
    }

    thread::spawn(move || {
        while rx.recv().is_ok() {
            while rx.recv_timeout(Duration::from_millis(1200)).is_ok() {}
            if let Some(state) = app.try_state::<crate::AppState>() {
                let scanned = {
                    let Ok(db) = state.db.lock() else {
                        continue;
                    };
                    scan_all(&db).is_ok()
                };
                if scanned {
                    let _ = app.emit("library-changed", ());
                }
            }
        }
    });

    Ok(watcher)
}

pub fn watch_folder(watcher: &mut RecommendedWatcher, path: &Path) {
    let _ = watcher.watch(path, RecursiveMode::Recursive);
}

pub fn unwatch_folder(watcher: &mut RecommendedWatcher, path: &Path) {
    let _ = watcher.unwatch(path);
}

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fs::Metadata;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use walkdir::WalkDir;

use crate::db::{Database, Upsert};
use crate::error::Error;
use crate::models::{Folder, ScanSummary};

const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "mov", "webm", "avi", "m4v", "wmv"];
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    "target",
    "$RECYCLE.BIN",
    "System Volume Information",
    "Windows",
    "Program Files",
    "Program Files (x86)",
    "AppData",
];
const MAX_DEPTH: usize = 8;
const MAX_VIDEOS: usize = 10_000;
const MAX_WARNINGS: usize = 5;

pub fn normalize_folder(path: &Path) -> Result<PathBuf, Error> {
    if path.as_os_str().is_empty() {
        return Err(Error::message("Pick a folder."));
    }
    if !path.exists() {
        return Err(Error::message("That folder does not exist."));
    }
    if !path.is_dir() {
        return Err(Error::message("Pick a folder, not a file."));
    }
    let canonical = match dunce::canonicalize(path) {
        Ok(path) => path,
        Err(err) if err.kind() == ErrorKind::PermissionDenied => {
            return Err(Error::message("RageTrim can't read that folder."));
        }
        Err(_) => return Err(Error::message("Couldn't open that folder.")),
    };
    if is_drive_root(&canonical) {
        return Err(Error::message("Pick a clip folder, not a whole drive."));
    }
    Ok(canonical)
}

pub fn nested_conflict(existing: &[Folder], candidate: &Path) -> Option<String> {
    existing.iter().find_map(|folder| {
        let other = Path::new(&folder.path);
        if paths_equal(other, candidate) {
            return None;
        }
        if path_is_inside(other, candidate) || path_is_inside(candidate, other) {
            Some(folder.path.clone())
        } else {
            None
        }
    })
}

pub fn scan_all(db: &Database) -> Result<ScanSummary, Error> {
    let mut summary = ScanSummary::empty();
    for folder in db.list_folders()? {
        summary.merge(scan_folder(db, &folder)?);
    }
    Ok(summary)
}

pub fn scan_folder(db: &Database, folder: &Folder) -> Result<ScanSummary, Error> {
    let root = PathBuf::from(&folder.path);
    if !root.is_dir() {
        let mut summary = ScanSummary::empty();
        push_warning(
            &mut summary,
            format!("{} is missing. Remove it or add it again.", root.display()),
        );
        return Ok(summary);
    }

    let mut summary = ScanSummary::empty();
    let mut seen = HashSet::new();
    let mut capped = false;
    let walker = WalkDir::new(&root)
        .follow_links(false)
        .max_depth(MAX_DEPTH)
        .into_iter()
        .filter_entry(|entry| entry.depth() == 0 || !is_skipped_dir(entry.file_name()));

    for entry in walker {
        if seen.len() >= MAX_VIDEOS {
            capped = true;
            push_warning(
                &mut summary,
                format!(
                    "Stopped after {MAX_VIDEOS} videos in {}. Narrow the folder if clips are missing.",
                    root.display()
                ),
            );
            break;
        }
        let entry = match entry {
            Ok(entry) => entry,
            Err(err) => {
                let path = err
                    .path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "a folder".to_string());
                push_warning(&mut summary, format!("Skipped {path}."));
                continue;
            }
        };
        if !entry.file_type().is_file() || !is_video_file(entry.path()) {
            continue;
        }
        if !entry.path().starts_with(&root) {
            summary.skipped += 1;
            continue;
        }
        let meta = match entry.metadata() {
            Ok(meta) => meta,
            Err(_) => {
                summary.skipped += 1;
                continue;
            }
        };
        if meta.len() == 0 {
            summary.skipped += 1;
            continue;
        }
        let path = entry.path().to_string_lossy().to_string();
        let file_name = entry.file_name().to_string_lossy().to_string();
        match db.upsert_clip(
            folder.id,
            &path,
            &file_name,
            meta.len() as i64,
            modified_ms(&meta),
        )? {
            Upsert::Added => summary.added += 1,
            Upsert::Updated => summary.updated += 1,
            Upsert::Unchanged => {}
        }
        seen.insert(path);
    }

    // A capped walk has not seen every file, so unmarked clips may still exist.
    if !capped {
        summary.marked_missing += db.mark_missing_unseen(folder.id, &seen)?;
    }
    Ok(summary)
}

pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            VIDEO_EXTENSIONS
                .iter()
                .any(|allowed| ext.eq_ignore_ascii_case(allowed))
        })
}

pub fn is_skipped_dir(name: &OsStr) -> bool {
    let name = name.to_string_lossy();
    name.starts_with('.') || SKIP_DIRS.iter().any(|skip| name.eq_ignore_ascii_case(skip))
}

pub fn is_drive_root(path: &Path) -> bool {
    let mut components = path.components();
    match (components.next(), components.next(), components.next()) {
        (Some(Component::Prefix(_)), Some(Component::RootDir), None) => true,
        (Some(Component::RootDir), None, _) => true,
        _ => false,
    }
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    path_components(left) == path_components(right)
}

fn path_is_inside(parent: &Path, child: &Path) -> bool {
    let parent = path_components(parent);
    let child = path_components(child);
    child.len() > parent.len() && child.starts_with(&parent)
}

fn path_components(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect()
}

fn modified_ms(meta: &Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map(|duration| duration.as_millis() as i64)
        .unwrap_or(0)
}

fn push_warning(summary: &mut ScanSummary, warning: String) {
    if summary.warnings.len() < MAX_WARNINGS {
        summary.warnings.push(warning);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    #[test]
    fn classifies_video_files_and_junk_dirs() {
        assert!(is_video_file(Path::new("loss.MP4")));
        assert!(is_video_file(Path::new(r"C:\Clips\round.mkv")));
        assert!(!is_video_file(Path::new("notes.txt")));
        assert!(!is_video_file(Path::new("clip")));
        assert!(is_skipped_dir(OsStr::new(".hidden")));
        assert!(is_skipped_dir(OsStr::new("node_modules")));
        assert!(!is_skipped_dir(OsStr::new("NVIDIA")));
    }

    #[cfg(windows)]
    #[test]
    fn rejects_a_drive_root_but_not_a_clip_folder() {
        assert!(is_drive_root(Path::new(r"C:\")));
        assert!(!is_drive_root(Path::new(r"C:\Videos\NVIDIA")));
    }

    #[test]
    fn nested_folders_conflict_case_insensitively() {
        let existing = vec![Folder {
            id: 1,
            path: r"C:\Videos\Clips".to_string(),
        }];
        assert!(nested_conflict(&existing, Path::new(r"C:\Videos\Clips\Monday")).is_some());
        assert!(nested_conflict(&existing, Path::new(r"C:\Videos")).is_some());
        assert!(nested_conflict(&existing, Path::new(r"c:\videos\clips")).is_none());
        assert!(nested_conflict(&existing, Path::new(r"D:\Clips")).is_none());
        assert!(nested_conflict(&existing, Path::new(r"C:\Videos\Clips2")).is_none());
    }

    #[test]
    fn scan_indexes_videos_and_marks_missing_files() {
        let dir = std::env::temp_dir().join(format!(
            "ragetrim-scan-{}-{}",
            std::process::id(),
            crate::db::now_ms()
        ));
        std::fs::create_dir_all(dir.join("kept")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden")).unwrap();
        std::fs::write(dir.join("kept").join("loss.mp4"), b"not-empty").unwrap();
        std::fs::write(dir.join("notes.txt"), b"skip").unwrap();
        std::fs::write(dir.join("empty.mp4"), b"").unwrap();
        std::fs::write(dir.join(".hidden").join("secret.mp4"), b"hidden").unwrap();

        let db_dir = dir.join("db");
        std::fs::create_dir_all(&db_dir).unwrap();
        let db = Database::open(&db_dir.join("test.db")).unwrap();
        let folder = db.add_folder(&dir.to_string_lossy()).unwrap();

        let summary = scan_folder(&db, &folder).unwrap();
        assert_eq!(summary.added, 1);
        assert_eq!(summary.skipped, 1);
        let clips = db.list_clips().unwrap();
        assert_eq!(clips.len(), 1);
        assert_eq!(clips[0].file_name, "loss.mp4");
        assert!(!clips[0].missing);

        std::fs::remove_file(dir.join("kept").join("loss.mp4")).unwrap();
        let summary = scan_folder(&db, &folder).unwrap();
        assert_eq!(summary.marked_missing, 1);
        assert!(db.list_clips().unwrap()[0].missing);

        drop(db);
        let _ = std::fs::remove_dir_all(dir);
    }
}

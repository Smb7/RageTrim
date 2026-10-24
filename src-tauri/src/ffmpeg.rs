use std::ffi::OsString;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::Error;

pub const DEFAULT_TARGET_BYTES: u64 = 19 * 1024 * 1024;
const AUDIO_BPS: u64 = 128_000;
const MIN_VIDEO_BPS: u64 = 180_000;
const MAX_VIDEO_BPS: u64 = 12_000_000;
const MAX_SELECTION_MS: i64 = 6 * 60 * 60 * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportMode {
    Copy,
    Compress,
}

impl ExportMode {
    pub fn parse(value: &str) -> Result<Self, Error> {
        match value {
            "copy" => Ok(Self::Copy),
            "compress" => Ok(Self::Compress),
            _ => Err(Error::message("Choose a fast cut or a Discord compress.")),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Compress => "compress",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    pub duration_ms: Option<i64>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub video_codec: Option<String>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JobProgress {
    clip_id: i64,
    phase: &'static str,
    ratio: f32,
}

pub fn validate_range(in_ms: i64, out_ms: i64, duration_ms: Option<i64>) -> Result<(), Error> {
    if in_ms < 0 || out_ms < 0 {
        return Err(Error::message("Trim points can't be negative."));
    }
    if out_ms - in_ms < 100 {
        return Err(Error::message("Select at least a tenth of a second."));
    }
    if out_ms - in_ms > MAX_SELECTION_MS {
        return Err(Error::message("That selection is longer than 6 hours."));
    }
    if let Some(duration) = duration_ms {
        if in_ms > duration + 500 {
            return Err(Error::message("The start is past the end of the clip."));
        }
        if out_ms > duration + 500 {
            return Err(Error::message("The end is past the end of the clip."));
        }
    }
    Ok(())
}

pub fn video_bitrate_bps(target_bytes: u64, duration_ms: i64) -> u64 {
    let duration_ms = duration_ms.max(1) as u64;
    let bits = target_bytes.saturating_mul(8).saturating_mul(92) / 100;
    let total_bps = bits.saturating_mul(1000) / duration_ms;
    total_bps
        .saturating_sub(AUDIO_BPS)
        .clamp(MIN_VIDEO_BPS, MAX_VIDEO_BPS)
}

pub fn output_height(source_height: Option<i64>, video_bps: u64) -> i32 {
    let source = source_height.unwrap_or(1080).clamp(2, 4320) as i32;
    let cap = if video_bps < 600_000 {
        480
    } else if video_bps < 1_500_000 {
        720
    } else {
        source
    };
    let height = source.min(cap).max(2);
    height - height % 2
}

pub fn export_file_name(file_name: &str, in_ms: i64, out_ms: i64, mode: ExportMode) -> String {
    let stem = safe_stem(file_name);
    match mode {
        // Stream copy into mp4 fails for mkv, webm, avi, and wmv. Keep a container that can hold the streams.
        ExportMode::Copy => format!("{stem}_{in_ms}-{out_ms}_cut.{}", copy_extension(file_name)),
        ExportMode::Compress => format!("{stem}_{in_ms}-{out_ms}_discord.mp4"),
    }
}

fn copy_extension(file_name: &str) -> &'static str {
    match Path::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "mp4" | "m4v" | "mov" => "mp4",
        "webm" => "webm",
        "mkv" => "mkv",
        _ => "mkv",
    }
}

pub fn find_tool(
    name: &str,
    configured: Option<&Path>,
    bundled_dirs: &[PathBuf],
) -> Option<PathBuf> {
    let file_name = tool_file_name(name);
    for dir in bundled_dirs {
        let candidate = dir.join(&file_name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    if let Some(path) = configured {
        let sibling = path.with_file_name(&file_name);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    if let Some(path) = which(name) {
        return Some(path);
    }
    app_tool(name).filter(|path| path.is_file())
}

pub fn bundled_tool_dirs(resource_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = resource_dir {
        push_install_dirs(&mut dirs, dir);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push_install_dirs(&mut dirs, dir);
            push_dev_dirs(&mut dirs, dir);
        }
    }
    dirs
}

fn push_install_dirs(dirs: &mut Vec<PathBuf>, base: &Path) {
    push_unique(dirs, base.join("ffmpeg"));
    push_unique(dirs, base.join("resources").join("ffmpeg"));
    push_unique(dirs, base.to_path_buf());
    push_unique(dirs, base.join("resources"));
}

fn push_dev_dirs(dirs: &mut Vec<PathBuf>, start: &Path) {
    let mut cursor = Some(start.to_path_buf());
    for _ in 0..6 {
        let Some(dir) = cursor else {
            break;
        };
        push_unique(dirs, dir.join("src-tauri").join("resources").join("ffmpeg"));
        cursor = dir.parent().map(|path| path.to_path_buf());
    }
}

fn push_unique(dirs: &mut Vec<PathBuf>, path: PathBuf) {
    if !dirs.iter().any(|dir| dir == &path) {
        dirs.push(path);
    }
}

pub fn export_args(
    input: &Path,
    output: &Path,
    in_ms: i64,
    out_ms: i64,
    mode: ExportMode,
    target_bytes: u64,
    height: Option<i64>,
) -> Vec<OsString> {
    // Input -ss is fast and may land on a keyframe. Output -t is a duration, so it stays
    // correct even when -to would be treated as relative to that seek.
    let mut args = vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-ss"),
        OsString::from(ms_arg(in_ms)),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-t"),
        OsString::from(ms_arg(out_ms.saturating_sub(in_ms))),
        OsString::from("-map"),
        OsString::from("0:v:0"),
        OsString::from("-map"),
        OsString::from("0:a:0?"),
        OsString::from("-sn"),
        OsString::from("-dn"),
    ];
    match mode {
        ExportMode::Copy => {
            args.extend([
                OsString::from("-c"),
                OsString::from("copy"),
                OsString::from("-avoid_negative_ts"),
                OsString::from("make_zero"),
            ]);
        }
        ExportMode::Compress => {
            let bps = video_bitrate_bps(target_bytes, out_ms - in_ms);
            let height = output_height(height, bps);
            args.extend([
                OsString::from("-vf"),
                OsString::from(format!("scale=-2:{height}")),
                OsString::from("-c:v"),
                OsString::from("libx264"),
                OsString::from("-preset"),
                OsString::from("veryfast"),
                OsString::from("-pix_fmt"),
                OsString::from("yuv420p"),
                OsString::from("-b:v"),
                OsString::from(bps.to_string()),
                OsString::from("-maxrate"),
                OsString::from(bps.to_string()),
                OsString::from("-bufsize"),
                OsString::from(bps.saturating_mul(2).to_string()),
                OsString::from("-c:a"),
                OsString::from("aac"),
                OsString::from("-b:a"),
                OsString::from("128k"),
                OsString::from("-movflags"),
                OsString::from("+faststart"),
            ]);
        }
    }
    args.push(OsString::from("-progress"));
    args.push(OsString::from("pipe:1"));
    args.push(OsString::from("-nostats"));
    args.push(output.as_os_str().to_os_string());
    args
}

pub fn thumbnail_args(input: &Path, output: &Path, start: &str) -> Vec<OsString> {
    vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-ss"),
        OsString::from(start),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-frames:v"),
        OsString::from("1"),
        OsString::from("-vf"),
        OsString::from("scale=320:-1"),
        output.as_os_str().to_os_string(),
    ]
}

pub fn run_quiet(tool: &Path, args: &[OsString]) -> Result<(), Error> {
    if !tool.is_file() {
        return Err(Error::message("video tools failed to load"));
    }
    let mut command = Command::new(tool);
    if let Some(dir) = tool.parent() {
        command.current_dir(dir);
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let status = command
        .status()
        .map_err(|_| Error::message("Couldn't start FFmpeg."))?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::message("FFmpeg could not make a thumbnail."))
    }
}

pub fn preview_args(input: &Path, output: &Path) -> Vec<OsString> {
    vec![
        OsString::from("-hide_banner"),
        OsString::from("-nostdin"),
        OsString::from("-y"),
        OsString::from("-i"),
        input.as_os_str().to_os_string(),
        OsString::from("-vf"),
        OsString::from("scale=-2:480"),
        OsString::from("-c:v"),
        OsString::from("libx264"),
        OsString::from("-preset"),
        OsString::from("veryfast"),
        OsString::from("-crf"),
        OsString::from("30"),
        OsString::from("-c:a"),
        OsString::from("aac"),
        OsString::from("-b:a"),
        OsString::from("96k"),
        OsString::from("-movflags"),
        OsString::from("+faststart"),
        OsString::from("-progress"),
        OsString::from("pipe:1"),
        OsString::from("-nostats"),
        output.as_os_str().to_os_string(),
    ]
}

pub fn parse_probe(json: &str) -> Result<Probe, Error> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|_| Error::message("FFprobe returned an unreadable report."))?;
    let streams = value
        .get("streams")
        .and_then(|streams| streams.as_array())
        .cloned()
        .unwrap_or_default();
    let video = streams
        .iter()
        .find(|stream| stream.get("codec_type").and_then(|kind| kind.as_str()) == Some("video"));
    let duration_ms = value
        .get("format")
        .and_then(|format| format.get("duration"))
        .and_then(seconds_to_ms)
        .or_else(|| {
            video
                .and_then(|stream| stream.get("duration"))
                .and_then(seconds_to_ms)
        });
    Ok(Probe {
        duration_ms,
        width: video.and_then(|stream| json_i64(stream.get("width"))),
        height: video.and_then(|stream| json_i64(stream.get("height"))),
        video_codec: video
            .and_then(|stream| stream.get("codec_name"))
            .and_then(|codec| codec.as_str())
            .map(str::to_string),
    })
}

pub fn run_tool(
    app: &AppHandle,
    tool: &Path,
    args: &[OsString],
    clip_id: i64,
    phase: &'static str,
    duration_ms: i64,
) -> Result<(), Error> {
    if !tool.is_file() {
        return Err(Error::message("video tools failed to load"));
    }
    let mut command = Command::new(tool);
    if let Some(dir) = tool.parent() {
        command.current_dir(dir);
    }
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command
        .spawn()
        .map_err(|_| Error::message("Couldn't start FFmpeg."))?;
    let stderr = child.stderr.take();
    let stderr_thread = thread::spawn(move || {
        let Some(stderr) = stderr else {
            return String::new();
        };
        let mut text = String::new();
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        while reader.read_line(&mut line).unwrap_or(0) > 0 {
            if text.len() < 4_000 {
                text.push_str(&line);
            }
            line.clear();
        }
        text
    });
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            if let Some(us) = progress_microseconds(&line) {
                let _ = app.emit(
                    "export-progress",
                    JobProgress {
                        clip_id,
                        phase,
                        ratio: progress_ratio(us, duration_ms),
                    },
                );
            }
        }
    }
    let status = child
        .wait()
        .map_err(|_| Error::message("FFmpeg stopped unexpectedly."))?;
    let stderr_text = stderr_thread.join().unwrap_or_default();
    if status.success() {
        return Ok(());
    }
    let detail = stderr_text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("FFmpeg failed.");
    Err(Error::message(format!(
        "FFmpeg failed: {}",
        detail.chars().take(240).collect::<String>()
    )))
}

pub fn probe_file(ffprobe: &Path, input: &Path) -> Result<Probe, Error> {
    if !ffprobe.is_file() {
        return Err(Error::message("video tools failed to load"));
    }
    let mut command = Command::new(ffprobe);
    if let Some(dir) = ffprobe.parent() {
        command.current_dir(dir);
    }
    command
        .arg("-v")
        .arg("error")
        .arg("-print_format")
        .arg("json")
        .arg("-show_format")
        .arg("-show_streams")
        .arg(input)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command
        .output()
        .map_err(|_| Error::message("Couldn't start FFprobe."))?;
    if !output.status.success() {
        return Err(Error::message("Couldn't read that clip."));
    }
    parse_probe(&String::from_utf8_lossy(&output.stdout))
}

fn progress_microseconds(line: &str) -> Option<u64> {
    // out_time_ms is a long-standing FFmpeg misnomer: it is microseconds, same as out_time_us.
    if let Some(raw) = line
        .strip_prefix("out_time_us=")
        .or_else(|| line.strip_prefix("out_time_ms="))
    {
        return raw.trim().parse().ok();
    }
    line.strip_prefix("out_time=")
        .and_then(|raw| parse_clock_us(raw.trim()))
}

fn parse_clock_us(value: &str) -> Option<u64> {
    let (hms, frac) = value.split_once('.').unwrap_or((value, "0"));
    let mut parts = hms.split(':');
    let hours: u64 = parts.next()?.parse().ok()?;
    let minutes: u64 = parts.next()?.parse().ok()?;
    let seconds: u64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || minutes >= 60 || seconds >= 60 {
        return None;
    }
    let digits: String = frac.chars().take(6).collect();
    let micros: u64 = format!("{digits:0<6}").parse().ok()?;
    Some((hours * 3600 + minutes * 60 + seconds) * 1_000_000 + micros)
}

fn progress_ratio(out_time_us: u64, duration_ms: i64) -> f32 {
    if duration_ms <= 0 {
        return 0.0;
    }
    let elapsed_ms = out_time_us / 1000;
    (elapsed_ms as f32 / duration_ms as f32).clamp(0.0, 1.0)
}

fn seconds_to_ms(value: &serde_json::Value) -> Option<i64> {
    let seconds = value
        .as_f64()
        .or_else(|| value.as_str()?.parse::<f64>().ok())?;
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as i64)
}

fn json_i64(value: Option<&serde_json::Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
            .or_else(|| value.as_str()?.parse().ok())
    })
}

fn ms_arg(ms: i64) -> String {
    format!("{:.3}", ms.max(0) as f64 / 1000.0)
}

fn safe_stem(file_name: &str) -> String {
    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("clip");
    let mut cleaned = String::new();
    for ch in stem.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ' ') {
            cleaned.push(ch);
        } else {
            cleaned.push('_');
        }
    }
    let cleaned = cleaned.trim().trim_matches('.').trim();
    let cleaned: String = cleaned.chars().take(60).collect();
    if cleaned.is_empty() {
        "clip".to_string()
    } else {
        cleaned
    }
}

fn tool_file_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(tool_file_name(name));
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn app_tool(name: &str) -> Option<PathBuf> {
    Some(
        crate::db::app_data_dir()
            .ok()?
            .join("ffmpeg")
            .join(tool_file_name(name)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_bitrate_stays_inside_the_target() {
        let bps = video_bitrate_bps(19 * 1024 * 1024, 30_000);
        assert!(bps >= MIN_VIDEO_BPS);
        assert!(bps <= MAX_VIDEO_BPS);
        let bits = (bps + AUDIO_BPS) as f64 * 30.0;
        assert!(bits / 8.0 < 19.0 * 1024.0 * 1024.0);
    }

    #[test]
    fn low_bitrate_scales_down() {
        assert_eq!(output_height(Some(1080), 400_000), 480);
        assert_eq!(output_height(Some(1080), 1_000_000), 720);
        assert_eq!(output_height(Some(1080), 4_000_000), 1080);
    }

    #[test]
    fn export_names_drop_path_characters() {
        assert_eq!(
            export_file_name(r"..\loss:final.mp4", 1000, 2500, ExportMode::Compress),
            "loss_final_1000-2500_discord.mp4"
        );
        let args = export_args(
            Path::new(r"C:\Clips\loss.mp4"),
            Path::new(r"C:\Videos\RageTrim\loss_cut.mp4"),
            1000,
            2500,
            ExportMode::Copy,
            DEFAULT_TARGET_BYTES,
            None,
        );
        assert!(args.iter().any(|arg| arg == "-c"));
        assert!(args.iter().any(|arg| arg == "copy"));
        assert!(args.iter().any(|arg| arg == "-t"));
        assert!(!args.iter().any(|arg| arg == "-to"));
        assert!(args.iter().any(|arg| arg == r"C:\Clips\loss.mp4"));
        assert!(!args.iter().any(|arg| arg.to_string_lossy().contains("&&")));
        assert_eq!(
            export_file_name("round.mkv", 0, 1500, ExportMode::Copy),
            "round_0-1500_cut.mkv"
        );
        assert_eq!(
            export_file_name("round.avi", 0, 1500, ExportMode::Copy),
            "round_0-1500_cut.mkv"
        );
    }

    #[test]
    fn progress_treats_ffmpeg_time_as_microseconds() {
        assert!((progress_ratio(5_000_000, 10_000) - 0.5).abs() < 0.01);
        // First second of a 10 minute encode must not jump to 100%.
        assert!(progress_ratio(1_000_000, 600_000) < 0.01);
        assert_eq!(parse_clock_us("00:00:05.500000"), Some(5_500_000));
        assert_eq!(
            progress_microseconds("out_time_ms=1500000"),
            Some(1_500_000)
        );
    }

    #[test]
    fn probe_reads_duration_and_codec() {
        let probe = parse_probe(
            r#"{"format":{"duration":"12.5"},"streams":[{"codec_type":"video","codec_name":"hevc","width":1920,"height":1080}]}"#,
        )
        .unwrap();
        assert_eq!(probe.duration_ms, Some(12_500));
        assert_eq!(probe.video_codec.as_deref(), Some("hevc"));
        assert_eq!(probe.height, Some(1080));
    }

    #[test]
    fn bundled_directory_is_used_before_a_saved_path() {
        let dir = std::env::temp_dir().join(format!("ragetrim-tool-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tool = dir.join(tool_file_name("ffmpeg"));
        std::fs::write(&tool, b"bundled").unwrap();
        let found = find_tool(
            "ffmpeg",
            Some(Path::new(r"C:\missing\ffmpeg.exe")),
            &[dir.clone()],
        );
        assert_eq!(found.as_deref(), Some(tool.as_path()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_a_tiny_or_backwards_range() {
        assert!(validate_range(1000, 1050, Some(10_000)).is_err());
        assert!(validate_range(5000, 1000, Some(10_000)).is_err());
        assert!(validate_range(0, 2000, Some(10_000)).is_ok());
    }
}

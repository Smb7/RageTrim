fn main() {
    fetch_ffmpeg();
    tauri_build::build();
}

fn fetch_ffmpeg() {
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let script = manifest_dir
        .join("..")
        .join("scripts")
        .join("fetch-ffmpeg.mjs");
    let status = std::process::Command::new("node")
        .arg(&script)
        .status()
        .unwrap_or_else(|err| {
            panic!(
                "failed to start node for the FFmpeg fetch ({}): {err}",
                script.display()
            )
        });
    if !status.success() {
        panic!(
            "FFmpeg fetch failed ({status}). The installer cannot ship without ffmpeg.exe and ffprobe.exe."
        );
    }
}

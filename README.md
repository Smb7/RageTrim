# RageTrim

Edit clips. Lose matches. Post pain to Discord.

## Download

[Download RageTrim 0.1.0 for Windows](https://github.com/Smb7/RageTrim/releases/download/v0.1.0/RageTrim_0.1.0_x64-setup.exe)

Run the installer, then open RageTrim from the Start menu. FFmpeg is included. You do not install FFmpeg, edit PATH, or choose ffmpeg.exe.

## Clip library

Local Windows app for browsing recorder clips, trimming them, and posting to Discord. Original files are never modified.

- Add one or more folders. Nested folders are rejected so the same clip is not indexed twice.
- Scan for `mp4`, `mkv`, `mov`, `webm`, `avi`, `m4v`, and `wmv`.
- Mark a clip viewed or new. Marks live in `%APPDATA%\RageTrim\ragetrim.db`, not next to the video.
- A missing file stays in the library until you remove the folder, so a viewed mark is not lost if the file is briefly gone.
- Trim a clip with in and out points. Fast cut keeps the original quality and may snap to a keyframe. It keeps an mp4, webm, or mkv container, and writes mkv for anything else so the copy is not forced into mp4. Discord compress re-encodes toward a size target, 19 MB by default.
- Exports go to `Videos\RageTrim` unless you pick another folder. Originals are never overwritten. A successful export marks the clip edited.
- The installer includes FFmpeg 9.0.2. RageTrim starts `ffmpeg.exe` and `ffprobe.exe` as separate programs. Codecs the preview cannot play get a temporary low-resolution proxy. FFmpeg is GPLv3 and is not covered by RageTrim's MIT license. Its license file is installed beside those programs.
- Post the latest export to a Discord channel webhook. The URL is stored in Windows Credential Manager, not in the library database. Only https webhook URLs on `discord.com`, `discordapp.com`, `ptb.discord.com`, or `canary.discord.com` are accepted.
- Filter the library by new, viewed, edited, or posted. New files in a watched folder show up after a short delay. Thumbnails are cached under app data.
- `npm run tauri build` downloads the pinned FFmpeg build if it is missing, then makes an NSIS installer. It embeds the WebView2 bootstrapper and installs the runtime if Windows does not already have it.

Drive roots are rejected. The scan skips hidden directories, `node_modules`, `AppData`, and other junk, and it stops after 10,000 videos in one folder.

## Develop

Requires Node, Rust, a Windows C++ toolchain, and the WebView2 runtime. The build downloads FFmpeg. You do not install it yourself.

```bash
npm install
npm run tauri dev
```

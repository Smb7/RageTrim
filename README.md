# RageTrim

Edit clips. Lose matches. Post pain to Discord.

Local Windows app for browsing recorder clips and marking what you have already watched. Trim and Discord posting come later. Original files are never modified.

## Clip library

- Add one or more folders. Nested folders are rejected so the same clip is not indexed twice.
- Scan for `mp4`, `mkv`, `mov`, `webm`, `avi`, `m4v`, and `wmv`.
- Mark a clip viewed or new. Marks live in `%APPDATA%\RageTrim\ragetrim.db`, not next to the video.
- A missing file stays in the library until you remove the folder, so a viewed mark is not lost if the file is briefly gone.
- Trim a clip with in and out points. Fast cut keeps the original quality and may snap to a keyframe. Discord compress re-encodes toward a size target, 19 MB by default.
- Exports go to `Videos\RageTrim` unless you pick another folder. Originals are never overwritten. A successful export marks the clip edited.
- FFmpeg must be on `PATH`, beside a path you choose, or in `%APPDATA%\RageTrim\ffmpeg`. Codecs the preview cannot play get a temporary low-resolution proxy.
- Post the latest export to a Discord channel webhook. The URL is stored in Windows Credential Manager, not in the library database, and only `https://discord.com` webhook URLs are accepted.
- Filter the library by new, viewed, edited, or posted. New files in a watched folder show up after a short delay. Thumbnails are cached under app data.
- `npm run tauri build` makes an NSIS installer. It embeds the WebView2 bootstrapper and installs the runtime if Windows does not already have it.

Drive roots are rejected. The scan skips hidden directories, `node_modules`, `AppData`, and other junk, and it stops after 10,000 videos in one folder.

## Develop

Requires Node, Rust, the MSVC C++ build tools, and the WebView2 runtime.

```bash
npm install
npm run tauri dev
```

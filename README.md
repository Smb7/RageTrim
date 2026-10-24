# RageTrim

Edit clips. Lose matches. Post pain to Discord.

Local Windows app for browsing recorder clips and marking what you have already watched. Trim and Discord posting come later. Original files are never modified.

## Clip library

- Add one or more folders. Nested folders are rejected so the same clip is not indexed twice.
- Scan for `mp4`, `mkv`, `mov`, `webm`, `avi`, `m4v`, and `wmv`.
- Mark a clip viewed or new. Marks live in `%APPDATA%\RageTrim\ragetrim.db`, not next to the video.
- A missing file stays in the library until you remove the folder, so a viewed mark is not lost if the file is briefly gone.

Drive roots are rejected. The scan skips hidden directories, `node_modules`, `AppData`, and other junk, and it stops after 10,000 videos in one folder.

## Develop

Requires Node, Rust, the MSVC C++ build tools, and the WebView2 runtime.

```bash
npm install
npm run tauri dev
```

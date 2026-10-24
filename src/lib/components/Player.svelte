<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import { errorText, exportClip, makePreview, postClip, probeClip, setCompressTarget } from "$lib/api";
  import { formatDuration } from "$lib/format";
  import type { AppSettings, Clip } from "$lib/types";

  let {
    clip,
    settings,
    onBack,
    onClip,
    onSettings,
  }: {
    clip: Clip;
    settings: AppSettings;
    onBack: () => void;
    onClip: (clip: Clip) => void;
    onSettings: (settings: AppSettings) => void;
  } = $props();

  const targets = [
    { label: "8 MB", bytes: 8 * 1024 * 1024 },
    { label: "10 MB", bytes: 10 * 1024 * 1024 },
    { label: "19 MB", bytes: 19 * 1024 * 1024 },
    { label: "25 MB", bytes: 25 * 1024 * 1024 },
    { label: "50 MB", bytes: 50 * 1024 * 1024 },
  ];

  let video = $state<HTMLVideoElement | null>(null);
  let mediaPath = $state("");
  let usingPreview = $state(false);
  let durationMs = $state(0);
  let inMs = $state(0);
  let outMs = $state(0);
  let mode = $state<"copy" | "compress">("compress");
  let busy = $state("");
  let error = $state("");
  let notice = $state("");
  let exportedPath = $state("");
  let postMessage = $state("");
  let progress = $state(0);
  let loadedId = 0;

  let selectionMs = $derived(Math.max(0, outMs - inMs));
  let src = $derived(mediaPath ? convertFileSrc(mediaPath) : "");
  let ffmpegReady = $derived(Boolean(settings.ffmpegPath && settings.ffprobePath));

  onMount(() => {
    const unlisten = listen<{ clipId: number; phase: string; ratio: number }>(
      "export-progress",
      (event) => {
        if (event.payload.clipId !== clip.id) return;
        progress = Math.round(event.payload.ratio * 100);
        busy = event.payload.phase === "preview" ? "Building preview…" : "Exporting…";
      },
    );
    return () => {
      void unlisten.then((stop) => stop());
    };
  });

  $effect(() => {
    if (loadedId === clip.id) return;
    loadedId = clip.id;
    mediaPath = clip.path;
    usingPreview = false;
    durationMs = clip.durationMs ?? 0;
    inMs = 0;
    outMs = clip.durationMs ?? 0;
    error = "";
    notice = "";
    exportedPath = "";
    postMessage = clip.fileName;
    progress = 0;
    void loadProbe(clip.id);
  });

  async function loadProbe(id: number) {
    try {
      const probed = await probeClip(id);
      if (loadedId !== id) return;
      onClip(probed);
      if (probed.durationMs) {
        durationMs = probed.durationMs;
        if (outMs === 0 || outMs > probed.durationMs) outMs = probed.durationMs;
      }
    } catch (err) {
      if (loadedId === id) notice = errorText(err);
    }
  }

  function onMeta() {
    if (!video || !Number.isFinite(video.duration) || video.duration <= 0) return;
    const fromVideo = Math.round(video.duration * 1000);
    if (durationMs <= 0) {
      durationMs = fromVideo;
      outMs = fromVideo;
    }
  }

  function onTime() {
    if (!video) return;
    if (video.currentTime * 1000 >= outMs && !video.paused) {
      video.pause();
    }
  }

  async function onVideoError() {
    if (usingPreview || !ffmpegReady || busy) return;
    busy = "Building preview…";
    error = "";
    try {
      const preview = await makePreview(clip.id);
      mediaPath = preview;
      usingPreview = true;
      notice = "Preview is a low-resolution copy. Export still uses the original.";
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = "";
    }
  }

  function setIn() {
    if (!video) return;
    inMs = Math.min(Math.round(video.currentTime * 1000), Math.max(0, outMs - 100));
  }

  function setOut() {
    if (!video) return;
    outMs = Math.max(Math.round(video.currentTime * 1000), inMs + 100);
  }

  function playSelection() {
    if (!video) return;
    video.currentTime = inMs / 1000;
    void video.play();
  }

  function clampIn(value: number) {
    inMs = Math.min(value, Math.max(0, outMs - 100));
  }

  function clampOut(value: number) {
    outMs = Math.max(value, inMs + 100);
  }

  async function changeTarget(event: Event) {
    const bytes = Number((event.currentTarget as HTMLSelectElement).value);
    try {
      onSettings(await setCompressTarget(bytes));
    } catch (err) {
      error = errorText(err);
    }
  }

  async function postExport() {
    error = "";
    busy = "Posting…";
    try {
      onClip(await postClip(clip.id, postMessage));
      notice = "Posted to Discord. The webhook URL stayed on this PC.";
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = "";
    }
  }

  async function exportSelection() {
    error = "";
    exportedPath = "";
    progress = 0;
    busy = "Exporting…";
    try {
      const result = await exportClip(clip.id, Math.round(inMs), Math.round(outMs), mode);
      onClip(result.clip);
      exportedPath = result.path;
      notice = mode === "copy"
        ? "Fast cut saved. It may snap to the nearest keyframe."
        : "Discord compress saved. The original was not changed.";
      postMessage = `${clip.fileName} ${formatDuration(inMs)}-${formatDuration(outMs)}`;
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = "";
    }
  }
</script>

<div class="player">
  <div class="player-head">
    <button class="ghost" type="button" onclick={onBack}>Back</button>
    <div>
      <h2>{clip.fileName}</h2>
      <p>
        {clip.videoCodec ?? "Unknown codec"}
        {#if clip.width && clip.height}
          · {clip.width}×{clip.height}
        {/if}
        · {formatDuration(durationMs)}
      </p>
    </div>
  </div>

  {#if error}
    <div class="banner" role="alert">{error}</div>
  {/if}
  {#if notice}
    <div class="warnings"><p>{notice}</p></div>
  {/if}

  <video
    bind:this={video}
    {src}
    controls
    preload="metadata"
    onloadedmetadata={onMeta}
    ontimeupdate={onTime}
    onerror={onVideoError}
  >
    <track kind="captions" />
  </video>

  <div class="trim">
    <label>
      In {formatDuration(inMs)}
      <input
        type="range"
        min="0"
        max={Math.max(durationMs, 1)}
        value={inMs}
        oninput={(event) => clampIn(Number(event.currentTarget.value))}
      />
    </label>
    <label>
      Out {formatDuration(outMs)}
      <input
        type="range"
        min="0"
        max={Math.max(durationMs, 1)}
        value={outMs}
        oninput={(event) => clampOut(Number(event.currentTarget.value))}
      />
    </label>
    <p class="muted">Selection {formatDuration(selectionMs)}</p>
    <div class="card-actions">
      <button class="ghost" type="button" onclick={setIn}>Set in</button>
      <button class="ghost" type="button" onclick={setOut}>Set out</button>
      <button class="ghost" type="button" onclick={playSelection}>Play selection</button>
    </div>
  </div>

  <div class="export-box">
    <label>
      Mode
      <select bind:value={mode}>
        <option value="compress">Discord compress</option>
        <option value="copy">Fast cut</option>
      </select>
    </label>
    {#if mode === "compress"}
      <label>
        Target
        <select value={settings.compressTargetBytes} onchange={changeTarget}>
          {#each targets as target (target.bytes)}
            <option value={target.bytes}>{target.label}</option>
          {/each}
        </select>
      </label>
    {:else}
      <p class="muted">Fast cut is quick and keeps the original quality, but it snaps to keyframes.</p>
    {/if}
    <button class="primary" type="button" disabled={Boolean(busy) || !ffmpegReady} onclick={exportSelection}>
      {busy ? `${busy}${progress ? ` ${progress}%` : ""}` : "Export"}
    </button>
    {#if !ffmpegReady}
      <p role="alert">video tools failed to load</p>
    {/if}
    {#if exportedPath}
      <code title={exportedPath}>{exportedPath}</code>
      <label>
        Discord message
        <textarea bind:value={postMessage} maxlength="2000" rows="3"></textarea>
      </label>
      <button class="primary" type="button" disabled={Boolean(busy)} onclick={postExport}>Post to Discord</button>
    {/if}
  </div>
</div>

<style>
  .player {
    display: flex;
    flex-direction: column;
    gap: 12px;
    min-height: 0;
    padding: 16px 20px 20px;
    overflow: auto;
  }

  .player-head {
    display: flex;
    align-items: center;
    gap: 12px;
  }

  .player-head h2,
  .player-head p {
    margin: 0;
  }

  .player-head p,
  code {
    color: #a39e94;
    font-size: 12px;
  }

  video {
    width: 100%;
    max-height: 420px;
    background: #000;
    border-radius: 10px;
  }

  .trim,
  .export-box {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    border: 1px solid #2a2d34;
    border-radius: 10px;
    background: #181b22;
  }

  label {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
    color: #a39e94;
  }

  input[type="range"],
  select {
    width: 100%;
  }

  select,
  textarea {
    border: 1px solid #3a3f4b;
    border-radius: 8px;
    background: #12141a;
    color: #f2efe9;
    padding: 6px 8px;
  }

  textarea {
    resize: vertical;
    font: inherit;
  }

  code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: Consolas, "Cascadia Mono", monospace;
  }
</style>

<script lang="ts">
  import { onMount } from "svelte";
  import Player from "$lib/components/Player.svelte";
  import {
    addFolder,
    errorText,
    getSettings,
    listClips,
    listFolders,
    pickFfmpeg,
    pickFolder,
    removeFolder,
    scanFolders,
    setEdited,
    setExportDir,
    setFfmpegPath,
    setViewed,
  } from "$lib/api";
  import { clipLocation, formatBytes, formatWhen, scanStatus, shortPath } from "$lib/format";
  import type { AppSettings, Clip, Folder, ScanSummary } from "$lib/types";

  let folders = $state<Folder[]>([]);
  let clips = $state<Clip[]>([]);
  let selectedId = $state<number | null>(null);
  let trimmingId = $state<number | null>(null);
  let settings = $state<AppSettings | null>(null);
  let busy = $state(false);
  let error = $state("");
  let summary = $state<ScanSummary | null>(null);

  let selected = $derived(clips.find((clip) => clip.id === selectedId) ?? null);
  let trimming = $derived(clips.find((clip) => clip.id === trimmingId) ?? null);
  let visibleCount = $derived(clips.filter((clip) => !clip.missing).length);
  let newCount = $derived(clips.filter((clip) => !clip.missing && clip.viewedAt == null).length);

  onMount(() => {
    void refresh();
  });

  async function refresh() {
    error = "";
    try {
      folders = await listFolders();
      clips = await listClips();
      settings = await getSettings();
      if (selectedId != null && !clips.some((clip) => clip.id === selectedId)) {
        selectedId = null;
      }
    } catch (err) {
      error = errorText(err);
    }
  }

  async function chooseFolder() {
    error = "";
    try {
      const path = await pickFolder();
      if (!path) return;
      busy = true;
      await addFolder(path);
      await scan();
    } catch (err) {
      error = errorText(err);
      busy = false;
    }
  }

  async function scan() {
    error = "";
    busy = true;
    try {
      summary = await scanFolders();
      await refresh();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function forgetFolder(folder: Folder) {
    error = "";
    busy = true;
    try {
      await removeFolder(folder.id);
      summary = null;
      await refresh();
    } catch (err) {
      error = errorText(err);
    } finally {
      busy = false;
    }
  }

  async function chooseExportDir() {
    error = "";
    try {
      const path = await pickFolder("Choose an export folder");
      if (!path) return;
      settings = await setExportDir(path);
    } catch (err) {
      error = errorText(err);
    }
  }

  async function locateFfmpeg() {
    error = "";
    try {
      const path = await pickFfmpeg();
      if (!path) return;
      settings = await setFfmpegPath(path);
    } catch (err) {
      error = errorText(err);
    }
  }

  function replaceClip(updated: Clip) {
    clips = clips.map((item) => (item.id === updated.id ? updated : item));
  }

  async function toggleViewed(clip: Clip) {
    error = "";
    try {
      const updated = await setViewed(clip.id, clip.viewedAt == null);
      replaceClip(updated);
    } catch (err) {
      error = errorText(err);
    }
  }

  async function toggleEdited(clip: Clip) {
    error = "";
    try {
      replaceClip(await setEdited(clip.id, clip.editedAt == null));
    } catch (err) {
      error = errorText(err);
    }
  }
</script>

<div class="shell">
  <header class="topbar">
    <div class="brand">
      <h1>RageTrim</h1>
      <p>Clip library</p>
    </div>
    <div class="topbar-actions">
      <button class="primary" type="button" disabled={busy || folders.length === 0} onclick={scan}>
        {busy ? "Working…" : "Scan clips"}
      </button>
    </div>
  </header>

  <div class="workspace">
    <aside class="sidebar">
      <div class="section-head">
        <h2>Folders</h2>
        <button class="ghost" type="button" disabled={busy} onclick={chooseFolder}>Add</button>
      </div>
      {#if folders.length === 0}
        <p class="muted">Add the folder your recorder saves into. Files stay where they are.</p>
      {:else}
        <ul class="folder-list">
          {#each folders as folder (folder.id)}
            <li class="folder">
              <div>
                <strong title={folder.path}>{shortPath(folder.path)}</strong>
                <span title={folder.path}>{folder.path}</span>
              </div>
              <button
                class="ghost danger"
                type="button"
                disabled={busy}
                title="Stop watching this folder. Clips stay on disk."
                onclick={() => forgetFolder(folder)}
              >
                Remove
              </button>
            </li>
          {/each}
        </ul>
      {/if}
      {#if settings}
        <div class="tool-box">
          <h2>Export</h2>
          <p title={settings.exportDir}>{shortPath(settings.exportDir)}</p>
          <button class="ghost" type="button" disabled={busy} onclick={chooseExportDir}>Change folder</button>
          <p>{settings.ffmpegPath ? "FFmpeg found" : "FFmpeg missing"}</p>
          <button class="ghost" type="button" disabled={busy} onclick={locateFfmpeg}>Locate FFmpeg</button>
        </div>
      {/if}
    </aside>

    <section class="library">
      <div class="library-head">
        <h2>Clips</h2>
        <p>{visibleCount} on disk · {newCount} new</p>
      </div>

      {#if error}
        <div class="banner" role="alert">{error}</div>
      {/if}
      {#if summary}
        <div class="warnings">
          <p>{scanStatus(summary)}</p>
          {#each summary.warnings as warning}
            <p>{warning}</p>
          {/each}
        </div>
      {/if}

      {#if trimming && settings}
        <Player
          clip={trimming}
          {settings}
          onBack={() => (trimmingId = null)}
          onClip={replaceClip}
          onSettings={(next) => (settings = next)}
        />
      {:else if folders.length === 0}
        <div class="empty">
          <strong>No folders yet</strong>
          Add a clip folder to start the library.
        </div>
      {:else if clips.length === 0}
        <div class="empty">
          <strong>No clips in the library</strong>
          Scan after a match, or pick a folder that already has videos.
        </div>
      {:else}
        <ul class="clip-grid">
          {#each clips as clip (clip.id)}
            <li class:selected={clip.id === selectedId} class:missing={clip.missing} class="card">
              <button class="card-select" type="button" onclick={() => (selectedId = clip.id)}>
                <h3 title={clip.fileName}>{clip.fileName}</h3>
                <p>
                  {clip.missing ? "Missing · " : ""}{formatBytes(clip.sizeBytes)} · {formatWhen(clip.modifiedMs)}
                </p>
                <p>{clipLocation(clip)}</p>
              </button>
              <div class="card-actions">
                <button
                  class="pill {clip.viewedAt == null ? 'new' : 'viewed'}"
                  type="button"
                  onclick={() => toggleViewed(clip)}
                >
                  {clip.viewedAt == null ? "Mark viewed" : "Viewed"}
                </button>
                <button
                  class="pill {clip.editedAt == null ? 'new' : 'viewed'}"
                  type="button"
                  onclick={() => toggleEdited(clip)}
                >
                  {clip.editedAt == null ? "Mark edited" : "Edited"}
                </button>
              </div>
            </li>
          {/each}
        </ul>
      {/if}

      {#if selected && !trimming}
        <div class="detail">
          <div>
            <strong>{selected.fileName}</strong>
            <code title={selected.path}>{selected.path}</code>
          </div>
          <div class="card-actions">
            <button class="ghost" type="button" onclick={() => toggleViewed(selected)}>
              {selected.viewedAt == null ? "Mark viewed" : "Mark as new"}
            </button>
            <button class="primary" type="button" disabled={selected.missing} onclick={() => (trimmingId = selected.id)}>
              Trim
            </button>
          </div>
        </div>
      {/if}
    </section>
  </div>
</div>

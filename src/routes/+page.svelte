<script lang="ts">
  import { listen } from "@tauri-apps/api/event";
  import { onMount } from "svelte";
  import Player from "$lib/components/Player.svelte";
  import Thumb from "$lib/components/Thumb.svelte";
  import {
    addFolder,
    errorText,
    clearDiscordWebhook,
    discordStatus,
    getSettings,
    listClips,
    listFolders,
    pickFolder,
    removeFolder,
    scanFolders,
    saveDiscordWebhook,
    setEdited,
    setPosted,
    setExportDir,
    setViewed,
    testDiscordWebhook,
  } from "$lib/api";
  import { clipLocation, formatBytes, formatWhen, scanStatus, shortPath } from "$lib/format";
  import type { AppSettings, Clip, DiscordStatus, Folder, ScanSummary } from "$lib/types";

  let folders = $state<Folder[]>([]);
  let clips = $state<Clip[]>([]);
  let selectedId = $state<number | null>(null);
  let trimmingId = $state<number | null>(null);
  let settings = $state<AppSettings | null>(null);
  let discord = $state<DiscordStatus>({ configured: false });
  let webhookUrl = $state("");
  let webhookNote = $state("");
  let busy = $state(false);
  let error = $state("");
  let summary = $state<ScanSummary | null>(null);
  const filters = ["all", "new", "viewed", "edited", "posted"] as const;
  let filter = $state<(typeof filters)[number]>("all");

  let selected = $derived(clips.find((clip) => clip.id === selectedId) ?? null);
  let trimming = $derived(clips.find((clip) => clip.id === trimmingId) ?? null);
  let visibleCount = $derived(clips.filter((clip) => !clip.missing).length);
  const filterLabels: Record<(typeof filters)[number], string> = {
    all: "All",
    new: "New",
    viewed: "Viewed",
    edited: "Edited",
    posted: "Posted",
  };

  function isNew(clip: Clip): boolean {
    return !clip.missing && clip.viewedAt == null && clip.editedAt == null && clip.postedAt == null;
  }

  let shown = $derived(
    clips.filter((clip) => {
      if (filter === "new") return isNew(clip);
      if (filter === "viewed") return clip.viewedAt != null;
      if (filter === "edited") return clip.editedAt != null;
      if (filter === "posted") return clip.postedAt != null;
      return true;
    }),
  );
  let newCount = $derived(clips.filter(isNew).length);

  onMount(() => {
    void refresh();
    const unlisten = listen("library-changed", () => {
      void refresh(true);
    });
    return () => {
      void unlisten.then((stop) => stop());
    };
  });

  async function refresh(quiet = false) {
    if (!quiet) error = "";
    try {
      folders = await listFolders();
      clips = await listClips();
      settings = await getSettings();
      discord = await discordStatus();
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

  async function saveWebhook() {
    error = "";
    webhookNote = "";
    try {
      discord = await saveDiscordWebhook(webhookUrl);
      webhookUrl = "";
      webhookNote = "Webhook saved in Windows Credential Manager.";
    } catch (err) {
      error = errorText(err);
    }
  }

  async function testWebhook() {
    error = "";
    webhookNote = "";
    try {
      const check = await testDiscordWebhook();
      webhookNote = `Webhook OK: ${check.name}`;
    } catch (err) {
      error = errorText(err);
    }
  }

  async function clearWebhook() {
    error = "";
    webhookNote = "";
    try {
      discord = await clearDiscordWebhook();
      webhookNote = "Webhook removed.";
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

  async function togglePosted(clip: Clip) {
    error = "";
    try {
      replaceClip(await setPosted(clip.id, clip.postedAt == null));
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
          {#if !settings.ffmpegPath || !settings.ffprobePath}
            <p role="alert">video tools failed to load</p>
          {/if}
        </div>
        <div class="tool-box">
          <h2>Discord</h2>
          <p>{discord.configured ? "Webhook saved" : "No webhook"}</p>
          <input
            type="password"
            autocomplete="off"
            spellcheck="false"
            placeholder="https://discord.com/api/webhooks/..."
            bind:value={webhookUrl}
          />
          <div class="card-actions">
            <button class="ghost" type="button" disabled={busy || webhookUrl.trim() === ""} onclick={saveWebhook}>
              Save
            </button>
            <button class="ghost" type="button" disabled={busy || !discord.configured} onclick={testWebhook}>
              Test
            </button>
            <button class="ghost danger" type="button" disabled={busy || !discord.configured} onclick={clearWebhook}>
              Clear
            </button>
          </div>
          {#if webhookNote}
            <p>{webhookNote}</p>
          {/if}
        </div>
      {/if}
    </aside>

    <section class="library">
      <div class="library-head">
        <h2>Clips</h2>
        <p>{visibleCount} on disk · {newCount} new</p>
      </div>
      <div class="filters">
        {#each filters as name (name)}
          <button class="ghost" class:active={filter === name} type="button" onclick={() => (filter = name)}>
            {filterLabels[name]}
          </button>
        {/each}
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
      {:else if shown.length === 0}
        <div class="empty">
          <strong>Nothing in this filter</strong>
          Try another filter, or scan again.
        </div>
      {:else}
        <ul class="clip-grid">
          {#each shown as clip (clip.id)}
            <li class:selected={clip.id === selectedId} class:missing={clip.missing} class="card">
              <button class="card-select" type="button" onclick={() => (selectedId = clip.id)}>
                {#key `${clip.id}-${clip.modifiedMs}`}
                  <Thumb {clip} />
                {/key}
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
                <button
                  class="pill {clip.postedAt == null ? 'new' : 'viewed'}"
                  type="button"
                  onclick={() => togglePosted(clip)}
                >
                  {clip.postedAt == null ? "Mark posted" : "Posted"}
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

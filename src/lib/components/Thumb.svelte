<script lang="ts">
  import { convertFileSrc } from "@tauri-apps/api/core";
  import { onMount } from "svelte";
  import { ensureThumbnail } from "$lib/api";
  import type { Clip } from "$lib/types";

  let { clip }: { clip: Clip } = $props();
  let src = $state("");

  onMount(() => {
    let cancelled = false;
    void ensureThumbnail(clip.id).then((path) => {
      if (!cancelled && path) src = convertFileSrc(path);
    });
    return () => {
      cancelled = true;
    };
  });
</script>

{#if src}
  <img {src} alt="" />
{:else}
  <div class="thumb-fallback" aria-hidden="true"></div>
{/if}

<style>
  img,
  .thumb-fallback {
    width: 100%;
    height: 112px;
    object-fit: cover;
    border-radius: 8px;
    background: #101218;
  }
</style>

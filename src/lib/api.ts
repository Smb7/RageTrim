import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  AppSettings,
  Clip,
  DiscordStatus,
  ExportResult,
  Folder,
  ScanSummary,
  WebhookCheck,
} from "./types";

export function listFolders(): Promise<Folder[]> {
  return invoke("list_folders");
}

export function addFolder(path: string): Promise<Folder> {
  return invoke("add_folder", { path });
}

export function removeFolder(id: number): Promise<void> {
  return invoke("remove_folder", { id });
}

export function scanFolders(): Promise<ScanSummary> {
  return invoke("scan_folders");
}

export function listClips(): Promise<Clip[]> {
  return invoke("list_clips");
}

export function ensureThumbnail(id: number): Promise<string | null> {
  return invoke("ensure_thumbnail", { id });
}

export function setViewed(id: number, viewed: boolean): Promise<Clip> {
  return invoke("set_viewed", { id, viewed });
}

export function setEdited(id: number, edited: boolean): Promise<Clip> {
  return invoke("set_edited", { id, edited });
}

export function setPosted(id: number, posted: boolean): Promise<Clip> {
  return invoke("set_posted", { id, posted });
}

export function discordStatus(): Promise<DiscordStatus> {
  return invoke("discord_status");
}

export function saveDiscordWebhook(url: string): Promise<DiscordStatus> {
  return invoke("save_discord_webhook", { url });
}

export function clearDiscordWebhook(): Promise<DiscordStatus> {
  return invoke("clear_discord_webhook");
}

export function testDiscordWebhook(): Promise<WebhookCheck> {
  return invoke("test_discord_webhook");
}

export function postClip(id: number, message: string): Promise<Clip> {
  return invoke("post_clip", { id, message });
}

export function getSettings(): Promise<AppSettings> {
  return invoke("get_settings");
}

export function setExportDir(path: string): Promise<AppSettings> {
  return invoke("set_export_dir", { path });
}

export function setCompressTarget(bytes: number): Promise<AppSettings> {
  return invoke("set_compress_target", { bytes });
}

export function probeClip(id: number): Promise<Clip> {
  return invoke("probe_clip", { id });
}

export function makePreview(id: number): Promise<string> {
  return invoke("make_preview", { id });
}

export function exportClip(
  id: number,
  inMs: number,
  outMs: number,
  mode: "copy" | "compress",
): Promise<ExportResult> {
  return invoke("export_clip", { id, inMs, outMs, mode });
}

export async function pickFolder(title = "Choose a clip folder"): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title,
  });
  return typeof selected === "string" ? selected : null;
}

export function errorText(err: unknown): string {
  if (typeof err === "string" && err.trim()) return err;
  if (err instanceof Error && err.message.trim()) return err.message;
  return "Something went wrong.";
}

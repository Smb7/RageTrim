import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { Clip, Folder, ScanSummary } from "./types";

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

export function setViewed(id: number, viewed: boolean): Promise<Clip> {
  return invoke("set_viewed", { id, viewed });
}

export async function pickFolder(): Promise<string | null> {
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Choose a clip folder",
  });
  return typeof selected === "string" ? selected : null;
}

export function errorText(err: unknown): string {
  if (typeof err === "string" && err.trim()) return err;
  if (err instanceof Error && err.message.trim()) return err.message;
  return "Something went wrong.";
}

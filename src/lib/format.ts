import type { Clip } from "./types";

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "—";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = value >= 10 || unit === 0 ? 0 : 1;
  return `${value.toFixed(digits)} ${units[unit]}`;
}

export function formatWhen(ms: number): string {
  if (!ms) return "Unknown date";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit",
  }).format(new Date(ms));
}

export function shortPath(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean);
  if (parts.length <= 2) return parts.join("\\");
  return parts.slice(-2).join("\\");
}

export function clipLocation(clip: Clip): string {
  const folder = clip.folderPath.replaceAll("\\", "/").replace(/\/$/, "");
  const file = clip.path.replaceAll("\\", "/");
  if (file.toLowerCase().startsWith(folder.toLowerCase())) {
    const rest = file.slice(folder.length).replace(/^\//, "");
    const slash = rest.lastIndexOf("/");
    if (slash > 0) return rest.slice(0, slash);
  }
  return shortPath(clip.folderPath);
}

export function scanStatus(summary: {
  added: number;
  updated: number;
  markedMissing: number;
}): string {
  const parts = [
    `${summary.added} added`,
    `${summary.updated} updated`,
    `${summary.markedMissing} missing`,
  ];
  return parts.join(" · ");
}

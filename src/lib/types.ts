export type Folder = {
  id: number;
  path: string;
};

export type Clip = {
  id: number;
  folderId: number;
  folderPath: string;
  path: string;
  fileName: string;
  sizeBytes: number;
  modifiedMs: number;
  viewedAt: number | null;
  missing: boolean;
  durationMs: number | null;
  width: number | null;
  height: number | null;
  videoCodec: string | null;
  editedAt: number | null;
};

export type AppSettings = {
  exportDir: string;
  compressTargetBytes: number;
  ffmpegPath: string | null;
  ffprobePath: string | null;
};

export type ExportResult = {
  path: string;
  clip: Clip;
};

export type ScanSummary = {
  added: number;
  updated: number;
  markedMissing: number;
  skipped: number;
  warnings: string[];
};

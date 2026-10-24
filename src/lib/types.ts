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
};

export type ScanSummary = {
  added: number;
  updated: number;
  markedMissing: number;
  skipped: number;
  warnings: string[];
};

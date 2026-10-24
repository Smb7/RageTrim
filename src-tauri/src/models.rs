use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: i64,
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: i64,
    pub folder_id: i64,
    pub folder_path: String,
    pub path: String,
    pub file_name: String,
    pub size_bytes: i64,
    pub modified_ms: i64,
    pub viewed_at: Option<i64>,
    pub missing: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    pub added: u32,
    pub updated: u32,
    pub marked_missing: u32,
    pub skipped: u32,
    pub warnings: Vec<String>,
}

impl ScanSummary {
    pub fn empty() -> Self {
        Self {
            added: 0,
            updated: 0,
            marked_missing: 0,
            skipped: 0,
            warnings: Vec::new(),
        }
    }

    pub fn merge(&mut self, other: ScanSummary) {
        self.added += other.added;
        self.updated += other.updated;
        self.marked_missing += other.marked_missing;
        self.skipped += other.skipped;
        self.warnings.extend(other.warnings);
    }
}

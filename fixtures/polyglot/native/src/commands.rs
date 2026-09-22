use crate::jobs::sync_jobs;

#[tauri::command]
pub fn scan_repo(path: String) -> String {
    sync_jobs();
    std::fs::read_to_string(&path).unwrap_or_default()
}

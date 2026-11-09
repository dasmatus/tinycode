use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub provider: String,
    pub model: String,
    pub base_url: String,
    pub root: String,
    pub turns: u32,
    #[serde(default)]
    pub messages: Vec<Value>,
}

pub fn new_path(root: &Path) -> Result<PathBuf, String> {
    let dir = root.join(".tinycode").join("sessions");
    fs::create_dir_all(&dir).map_err(|e| format!("session dir {}: {e}", dir.display()))?;
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::SystemTime::UNIX_EPOCH)
        .map_err(|e| format!("clock: {e}"))?
        .as_secs();
    Ok(dir.join(format!("tc-{:x}-{}.json", secs, std::process::id())))
}

pub fn load(path: &Path) -> Result<Session, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("session {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("session {}: {e}", path.display()))
}

/// Write to a temporary file first, then rename, so a crash never leaves a
/// torn session file.
pub fn save(path: &Path, session: &Session) -> Result<(), String> {
    let data = serde_json::to_vec(session).map_err(|e| format!("session encode: {e}"))?;
    // The agent's own run tool can take the sessions directory away mid-run
    // (a `git clean -fdx`-style tidy-up sweeps untracked state, this
    // directory included); a save recreates it instead of losing the
    // session to it.
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("session dir {}: {e}", dir.display()))?;
    }
    let tmp = path.with_extension(format!("tmp.{}", std::process::id()));
    fs::write(&tmp, &data).map_err(|e| format!("session {}: {e}", tmp.display()))?;
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("session {}: {e}", path.display())
    })
}

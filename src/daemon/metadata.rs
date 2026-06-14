use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use super::session::{Session, SessionRegistry};
use super::state;

/// Persistent session metadata record.
/// This is what gets written to sessions.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub pid: Option<u32>,
    pub status: String,
    pub exit_code: Option<i32>,
    pub created_at: u64,
    pub updated_at: u64,
    pub log_path: Option<String>,
}

impl From<&Session> for SessionMetadata {
    fn from(s: &Session) -> Self {
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            profile: s.profile.clone(),
            command: s.command.clone(),
            args: s.args.clone(),
            cwd: s.cwd.clone(),
            pid: s.pid,
            status: s.status.to_string(),
            exit_code: s.exit_code,
            created_at: s.created_at,
            updated_at: s.updated_at,
            log_path: s.log_path.as_ref().map(|p| p.to_string_lossy().to_string()),
        }
    }
}

/// Get the path to sessions.json in the data directory.
pub fn sessions_file_path() -> Result<PathBuf> {
    Ok(state::socket_dir()?.join("sessions.json"))
}

/// Collect metadata from all sessions in the registry.
fn collect_metadata(registry: &SessionRegistry) -> Vec<SessionMetadata> {
    registry
        .list()
        .iter()
        .map(|s| SessionMetadata::from(*s))
        .collect()
}

/// Write sessions metadata to sessions.json atomically.
///
/// Writes to a temp file first, then renames to avoid corruption.
pub fn save_metadata(registry: &SessionRegistry) -> Result<()> {
    let path = sessions_file_path()?;
    let data = collect_metadata(registry);
    let json = serde_json::to_string_pretty(&data)?;

    // Write to temp file in the same directory (required for atomic rename).
    let dir = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot determine parent dir"))?;
    let temp_path = dir.join(format!(".sessions.json.tmp.{}", std::process::id()));

    let mut file = std::fs::File::create(&temp_path)?;
    file.write_all(json.as_bytes())?;
    file.write_all(b"\n")?;
    // fsync for durability.
    let _ = file.sync_all();
    drop(file);

    // Atomic rename.
    std::fs::rename(&temp_path, &path)?;
    Ok(())
}

/// Load sessions.json and return parsed metadata records.
///
/// If the file doesn't exist, returns empty vec.
/// If the file is invalid JSON, backs it up as .corrupt.<timestamp> and returns empty vec.
pub fn load_metadata() -> Result<Vec<SessionMetadata>> {
    let path = sessions_file_path()?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("Failed to read sessions.json: {}", e);
            return Ok(Vec::new());
        }
    };

    match serde_json::from_str::<Vec<SessionMetadata>>(&content) {
        Ok(data) => Ok(data),
        Err(e) => {
            tracing::warn!("Invalid sessions.json: {}. Backing up.", e);
            let ts = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let backup = path.with_extension(format!("corrupt.{}", ts));
            let _ = std::fs::rename(&path, &backup);
            tracing::info!("Backed up corrupt sessions.json to {}", backup.display());
            Ok(Vec::new())
        }
    }
}

/// Load sessions.json, classify each session's PID as alive or dead,
/// and return recovered sessions with appropriate statuses.
///
/// - Alive PID → Orphaned (process may exist but daemon doesn't own the PTY)
/// - Dead PID → Exited
pub fn recover_sessions() -> Result<Vec<SessionMetadata>> {
    let sessions = load_metadata()?;
    let mut recovered = Vec::new();

    for mut meta in sessions {
        let alive = meta.pid.map(state::is_pid_alive).unwrap_or(false);

        if alive {
            meta.status = "orphaned".to_string();
        } else {
            meta.status = "exited".to_string();
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        meta.updated_at = now;

        recovered.push(meta);
    }

    Ok(recovered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::OnceLock;

    static TEST_LOCK: OnceLock<std::sync::Mutex<()>> = OnceLock::new();

    fn get_lock() -> &'static std::sync::Mutex<()> {
        TEST_LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }

    fn setup_temp_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("AGENTMUX_DATA_DIR", dir.path());
        dir
    }

    #[test]
    fn metadata_serialization_roundtrip() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        let meta = SessionMetadata {
            id: "test-id-123".to_string(),
            name: "test-session".to_string(),
            profile: "shell".to_string(),
            command: "bash".to_string(),
            args: vec!["-l".to_string()],
            cwd: Some("/tmp".to_string()),
            pid: Some(12345),
            status: "running".to_string(),
            exit_code: None,
            created_at: 1700000000,
            updated_at: 1700000001,
            log_path: Some("/tmp/test.log".to_string()),
        };

        let json = serde_json::to_string(&meta).unwrap();
        let deserialized: SessionMetadata = serde_json::from_str(&json).unwrap();

        assert_eq!(deserialized.id, "test-id-123");
        assert_eq!(deserialized.name, "test-session");
        assert_eq!(deserialized.profile, "shell");
        assert_eq!(deserialized.command, "bash");
        assert_eq!(deserialized.args, vec!["-l".to_string()]);
        assert_eq!(deserialized.cwd, Some("/tmp".to_string()));
        assert_eq!(deserialized.pid, Some(12345));
        assert_eq!(deserialized.status, "running");
        assert_eq!(deserialized.created_at, 1700000000);
        assert_eq!(deserialized.updated_at, 1700000001);
        assert_eq!(deserialized.log_path, Some("/tmp/test.log".to_string()));
    }

    #[test]
    fn atomic_write_success() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        // Create a registry with a session.
        let mut reg = SessionRegistry::new();
        let session = Session::new("test-atomic", "shell", "bash", vec![], None).unwrap();
        reg.add(session).unwrap();

        // Save.
        save_metadata(&reg).unwrap();

        // Read back.
        let loaded = load_metadata().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "test-atomic");
        assert_eq!(loaded[0].command, "bash");
    }

    #[test]
    fn corrupt_json_backup() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        // Write invalid JSON to sessions.json.
        let path = sessions_file_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ this is not valid json").unwrap();
        assert!(path.exists());

        // Load should detect corruption and back it up.
        let loaded = load_metadata().unwrap();
        assert!(loaded.is_empty());

        // Original should be gone (renamed to .corrupt.*).
        assert!(!path.exists());

        // A corrupt backup should exist.
        let parent = path.parent().unwrap();
        let has_corrupt = std::fs::read_dir(parent)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.file_name().to_string_lossy().contains("corrupt"));
        assert!(has_corrupt, "Expected a .corrupt backup file");
    }

    #[test]
    fn missing_file_returns_empty() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        // Ensure no file exists.
        let path = sessions_file_path().unwrap();
        let _ = std::fs::remove_file(&path);

        let loaded = load_metadata().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn recover_dead_pid_marked_exited() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        // Write metadata with a PID that definitely doesn't exist (999999).
        let meta = vec![SessionMetadata {
            id: "test-dead".to_string(),
            name: "dead-session".to_string(),
            profile: "shell".to_string(),
            command: "bash".to_string(),
            args: vec![],
            cwd: None,
            pid: Some(999999),
            status: "running".to_string(),
            exit_code: None,
            created_at: 1700000000,
            updated_at: 1700000000,
            log_path: None,
        }];

        let path = sessions_file_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

        let recovered = recover_sessions().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, "exited");
    }

    #[test]
    fn recover_live_pid_marked_orphaned() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        // Use our own process PID — it's alive.
        let my_pid = std::process::id();
        let meta = vec![SessionMetadata {
            id: "test-live".to_string(),
            name: "live-session".to_string(),
            profile: "shell".to_string(),
            command: "bash".to_string(),
            args: vec![],
            cwd: None,
            pid: Some(my_pid),
            status: "running".to_string(),
            exit_code: None,
            created_at: 1700000000,
            updated_at: 1700000000,
            log_path: None,
        }];

        let path = sessions_file_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&meta).unwrap()).unwrap();

        let recovered = recover_sessions().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, "orphaned");
    }

    #[test]
    fn multiple_sessions_persist_and_load() {
        let _lock = get_lock().lock().unwrap();
        let _dir = setup_temp_dir();

        let mut reg = SessionRegistry::new();
        reg.add(Session::new("s1", "shell", "bash", vec![], None).unwrap())
            .unwrap();
        reg.add(Session::new("s2", "shell", "vim", vec![], None).unwrap())
            .unwrap();
        reg.add(Session::new("s3", "shell", "top", vec![], None).unwrap())
            .unwrap();

        save_metadata(&reg).unwrap();
        let loaded = load_metadata().unwrap();
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].name, "s1");
        assert_eq!(loaded[1].name, "s2");
        assert_eq!(loaded[2].name, "s3");
    }
}

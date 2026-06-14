use anyhow::Result;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Maximum log file size before rotation is triggered (10 MB).
pub const MAX_LOG_SIZE: u64 = 10 * 1024 * 1024;

/// Maximum number of rotated files to keep (log.1, log.2, log.3).
pub const MAX_ROTATED_FILES: usize = 3;

/// Check rotation after roughly this many newly-written bytes.
/// This avoids stat() on every tiny write while still rotating promptly.
const ROTATION_CHECK_INTERVAL_BYTES: u64 = 64 * 1024;

/// Get the logs directory: ~/.local/share/agentmux/logs/
fn logs_dir() -> Result<PathBuf> {
    let dir = crate::daemon::state::socket_dir()?.join("logs");
    if !dir.exists() {
        std::fs::create_dir_all(&dir)?;
    }
    Ok(dir)
}

/// Get the log file path for a session name.
pub fn log_file_path(session_name: &str) -> PathBuf {
    // Sanitize session name for filesystem safety.
    let safe_name = session_name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>();
    match logs_dir() {
        Ok(dir) => dir.join(format!("{}.log", safe_name)),
        Err(_) => PathBuf::from(format!("/tmp/agentmux-{}.log", safe_name)),
    }
}

/// Same as [`log_file_path`]; kept as a separate name for clarity at call sites
/// that intend to use rotation alongside it.
pub fn log_file_path_with_rotation(session_name: &str) -> PathBuf {
    log_file_path(session_name)
}

/// Check if the log file at `log_path` exceeds [`MAX_LOG_SIZE`].
/// If so, rotate it: `log` → `log.1`, `log.1` → `log.2`, …, `log.(N-1)` → `log.N`,
/// and delete `log.N` (oldest). Keeps at most [`MAX_ROTATED_FILES`] rotated copies.
///
/// Returns `Ok(())` even if individual rotation steps fail — errors are logged
/// via `tracing::warn!` so the caller never crashes on rotation.
pub fn rotate_log_if_needed(log_path: &Path) -> Result<()> {
    // No file or file under the size threshold → nothing to do.
    let needs_rotation = match std::fs::metadata(log_path) {
        Ok(meta) => meta.len() > MAX_LOG_SIZE,
        Err(_) => false, // file doesn't exist — nothing to rotate
    };
    if !needs_rotation {
        return Ok(());
    }

    // First, delete any rotated files beyond the max (e.g. log.4, log.5, ...).
    let mut extra = MAX_ROTATED_FILES + 1;
    loop {
        let extra_path = append_suffix(log_path, extra);
        if extra_path.exists() {
            if let Err(e) = std::fs::remove_file(&extra_path) {
                tracing::warn!(
                    "Failed to delete excess rotated log {}: {}",
                    extra_path.display(),
                    e
                );
            }
            extra += 1;
        } else {
            break;
        }
    }

    // Rotate from highest index down to 1 so we don't overwrite before moving.
    //   log.(N-1) → log.N
    //   ...
    //   log.1    → log.2
    //   log      → log.1
    // At each step, if dst already exists, delete it first.
    for i in (1..=MAX_ROTATED_FILES).rev() {
        let src = if i == 1 {
            log_path.to_path_buf()
        } else {
            append_suffix(log_path, i - 1)
        };
        let dst = append_suffix(log_path, i);

        if !src.exists() {
            continue;
        }

        // Delete destination if it exists (handles the oldest file being evicted).
        if dst.exists() {
            if let Err(e) = std::fs::remove_file(&dst) {
                tracing::warn!("Failed to delete old rotated log {}: {}", dst.display(), e);
                continue;
            }
        }

        if let Err(e) = std::fs::rename(&src, &dst) {
            tracing::warn!(
                "Failed to rotate {} → {}: {}",
                src.display(),
                dst.display(),
                e
            );
        }
    }

    Ok(())
}

/// A log writer that periodically checks for size-based rotation.
///
/// It writes to the active `session.log`, and when rotation triggers it:
/// 1. flushes and drops the current file handle
/// 2. rotates `session.log` → `.1`, `.1` → `.2`, `.2` → `.3`
/// 3. reopens a fresh `session.log`
///
/// Errors only emit warnings and never crash the session.
pub struct RotatingLogWriter {
    log_path: PathBuf,
    file: std::fs::File,
    bytes_since_check: u64,
}

impl RotatingLogWriter {
    pub fn new(log_path: &Path) -> Result<Self> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(log_path)?;
        Ok(Self {
            log_path: log_path.to_path_buf(),
            file,
            bytes_since_check: 0,
        })
    }

    pub fn write_chunk(&mut self, chunk: &[u8]) {
        if let Err(e) = self.file.write_all(chunk) {
            tracing::warn!("Failed to write to log {}: {}", self.log_path.display(), e);
            return;
        }
        let _ = self.file.flush();

        self.bytes_since_check += chunk.len() as u64;
        if self.bytes_since_check >= ROTATION_CHECK_INTERVAL_BYTES {
            self.bytes_since_check = 0;
            self.rotate_if_needed();
        }
    }

    fn rotate_if_needed(&mut self) {
        let needs_rotation = match std::fs::metadata(&self.log_path) {
            Ok(meta) => meta.len() > MAX_LOG_SIZE,
            Err(_) => false,
        };
        if !needs_rotation {
            return;
        }

        let _ = self.file.flush();

        let reopen = || -> Result<std::fs::File> {
            Ok(std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)?)
        };

        match reopen() {
            Ok(replacement) => {
                let old_file = std::mem::replace(&mut self.file, replacement);
                drop(old_file);
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to prepare log rotation for {}: {}",
                    self.log_path.display(),
                    e
                );
                return;
            }
        }

        if let Err(e) = rotate_log_if_needed(&self.log_path) {
            tracing::warn!("Log rotation failed for {}: {}", self.log_path.display(), e);
        }

        match reopen() {
            Ok(new_file) => {
                self.file = new_file;
            }
            Err(e) => {
                tracing::warn!(
                    "Failed to reopen active log {} after rotation: {}",
                    self.log_path.display(),
                    e
                );
            }
        }
    }
}

/// Append a numeric suffix (e.g. `.1`) to a path, preserving the extension
/// position so `foo.log` becomes `foo.log.1`.
fn append_suffix(path: &Path, n: usize) -> PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(format!(".{}", n));
    PathBuf::from(s)
}

/// Read the last `n` bytes of a file, then return the lines.
/// Handles binary/invalid UTF-8 by using lossy conversion.
pub fn tail_log(path: &std::path::Path, tail_lines: usize) -> Result<Vec<String>> {
    if !path.exists() {
        anyhow::bail!("Log file not found: {}", path.display());
    }

    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);

    let mut lines: Vec<String> = Vec::new();
    for byte_result in reader.split(b'\n') {
        let bytes = byte_result.unwrap_or_default();
        // Use lossy UTF-8 conversion to handle binary/invalid data.
        let line = String::from_utf8_lossy(&bytes).into_owned();
        lines.push(line);
    }

    // Take last `tail_lines` lines.
    if lines.is_empty() {
        return Ok(vec![]);
    }

    // Remove trailing empty line if present (from final newline).
    if lines.last().map(|l| l.is_empty()).unwrap_or(false) {
        lines.pop();
    }

    let start = if lines.len() > tail_lines {
        lines.len() - tail_lines
    } else {
        0
    };
    Ok(lines[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Seek, SeekFrom};

    #[test]
    fn log_file_path_contains_session_name() {
        let path = log_file_path("test-session");
        assert!(path.to_string_lossy().ends_with("test-session.log"));
    }

    #[test]
    fn log_file_path_sanitizes_unsafe_chars() {
        let path = log_file_path("test/session");
        let name = path.to_string_lossy();
        assert!(name.contains("test_session.log"));
    }

    #[test]
    fn tail_log_handles_empty_file() {
        let dir = std::env::temp_dir();
        let path = dir.join("agentmux_test_empty.log");
        std::fs::write(&path, b"").unwrap();

        let lines = tail_log(&path, 10).unwrap();
        assert!(lines.is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tail_log_returns_last_n_lines() {
        let dir = std::env::temp_dir();
        let path = dir.join("agentmux_test_tail.log");
        let content = "line1\nline2\nline3\nline4\nline5\n";
        std::fs::write(&path, content).unwrap();

        let lines = tail_log(&path, 2).unwrap();
        assert_eq!(lines, vec!["line4", "line5"]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tail_log_handles_binary_data() {
        let dir = std::env::temp_dir();
        let path = dir.join("agentmux_test_binary.log");
        let content: Vec<u8> = vec![0x48, 0x65, 0x6c, 0x6c, 0x6f, 0x0a, 0xff, 0xfe, 0x0a];
        std::fs::write(&path, &content).unwrap();

        let lines = tail_log(&path, 10).unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "Hello");
        // Binary bytes should be lossy-converted (replacement chars)
        assert!(lines[1].contains('\u{fffd}') || !lines[1].is_empty());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tail_log_returns_error_for_nonexistent() {
        let result = tail_log(std::path::Path::new("/nonexistent/path/file.log"), 10);
        assert!(result.is_err());
    }

    #[test]
    fn tail_log_all_lines_when_fewer_than_tail() {
        let dir = std::env::temp_dir();
        let path = dir.join("agentmux_test_fewer.log");
        std::fs::write(&path, "a\nb\n").unwrap();

        let lines = tail_log(&path, 10).unwrap();
        assert_eq!(lines, vec!["a", "b"]);

        let _ = std::fs::remove_file(&path);
    }

    // ---- Log rotation tests ----

    /// Helper: write data of the given byte size to a path.
    fn make_file(path: &Path, size: u64) {
        let chunk = vec![b'x'; 4096];
        let mut remaining = size;
        let mut file = std::fs::File::create(path).unwrap();
        while remaining > 0 {
            let take = remaining.min(chunk.len() as u64) as usize;
            file.write_all(&chunk[..take]).unwrap();
            remaining -= take as u64;
        }
        file.sync_all().unwrap();
    }

    fn cleanup(paths: &[&Path]) {
        for p in paths {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn test_log_rotation_creates_numbered_files() {
        let dir = std::env::temp_dir();
        let log = dir.join("agentmux_rot_test1.log");
        let log1 = dir.join("agentmux_rot_test1.log.1");

        make_file(&log, MAX_LOG_SIZE + 1);

        rotate_log_if_needed(&log).unwrap();

        assert!(!log.exists(), "original log should have been renamed away");
        assert!(log1.exists(), "log.1 should exist after rotation");

        cleanup(&[&log, &log1]);
    }

    #[test]
    fn test_log_rotation_chain() {
        let dir = std::env::temp_dir();
        let log = dir.join("agentmux_rot_chain.log");
        let log1 = dir.join("agentmux_rot_chain.log.1");
        let log2 = dir.join("agentmux_rot_chain.log.2");
        let log3 = dir.join("agentmux_rot_chain.log.3");

        // Create log (> MAX), log.1 (> MAX), log.2 (> MAX) — all need rotation.
        make_file(&log, MAX_LOG_SIZE + 1);
        make_file(&log1, MAX_LOG_SIZE + 1);
        make_file(&log2, MAX_LOG_SIZE + 1);

        rotate_log_if_needed(&log).unwrap();

        // After rotation: log gone, log.1 = old log, log.2 = old log.1, log.3 = old log.2
        assert!(!log.exists(), "original should be gone");
        assert!(log1.exists(), "log.1 should exist");
        assert!(log2.exists(), "log.2 should exist");
        assert!(log3.exists(), "log.3 should exist (shifted from old log.2)");

        cleanup(&[&log, &log1, &log2, &log3]);
    }

    #[test]
    fn test_log_rotation_no_rotation_needed() {
        let dir = std::env::temp_dir();
        let log = dir.join("agentmux_rot_small.log");
        let log1 = dir.join("agentmux_rot_small.log.1");

        make_file(&log, 1024); // well under threshold

        rotate_log_if_needed(&log).unwrap();

        assert!(log.exists(), "file should still be there");
        assert!(!log1.exists(), "no .1 file should be created");

        cleanup(&[&log, &log1]);
    }

    #[test]
    fn test_log_rotation_missing_file_ok() {
        let dir = std::env::temp_dir();
        let log = dir.join("agentmux_rot_nonexistent.log");

        // Should not error on missing file.
        let result = rotate_log_if_needed(&log);
        assert!(result.is_ok());

        cleanup(&[&log]);
    }

    #[test]
    fn test_log_rotation_max_files_capped() {
        let dir = std::env::temp_dir();
        let base = "agentmux_rot_cap";
        let log = dir.join(format!("{}.log", base));
        // Pre-create .1, .2, .3, plus .4 (should not survive).
        let mut paths: Vec<PathBuf> = vec![log.clone()];
        for i in 1..=4 {
            paths.push(dir.join(format!("{}.log.{}", base, i)));
        }

        // Fill all files so every slot has data to rotate.
        for p in &paths {
            make_file(p, MAX_LOG_SIZE + 1);
        }

        rotate_log_if_needed(&log).unwrap();

        // log.4 should have been deleted during rotation (it's beyond MAX_ROTATED_FILES).
        assert!(
            !paths[4].exists(),
            "log.4 should not exist — capped at {} rotated files",
            MAX_ROTATED_FILES
        );
        // log.1 through log.3 should exist.
        for (i, p) in paths.iter().enumerate().take(4).skip(1) {
            assert!(p.exists(), "log.{} should exist after rotation", i);
        }

        for p in &paths {
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn test_log_rotation_preserves_content_order() {
        let dir = std::env::temp_dir();
        let base = "agentmux_rot_order";
        let log = dir.join(format!("{}.log", base));
        let log1 = dir.join(format!("{}.log.1", base));

        // Write a recognizable marker to log so we can verify it ends up in .1.
        std::fs::write(&log, "MARKER_NEWEST").unwrap();
        // Pad to exceed MAX_LOG_SIZE.
        {
            let mut f = std::fs::OpenOptions::new().write(true).open(&log).unwrap();
            f.seek(SeekFrom::Start(MAX_LOG_SIZE + 1)).unwrap();
            f.write_all(b"END").unwrap();
            f.sync_all().unwrap();
        }

        rotate_log_if_needed(&log).unwrap();

        assert!(log1.exists());
        let content = std::fs::read_to_string(&log1).unwrap();
        assert!(
            content.starts_with("MARKER_NEWEST"),
            "content should be preserved in .1"
        );

        cleanup(&[&log, &log1]);
    }

    #[test]
    fn rotating_log_writer_reopens_after_rotation() {
        let dir = std::env::temp_dir();
        let log = dir.join("agentmux_rot_writer.log");
        let log1 = dir.join("agentmux_rot_writer.log.1");

        let mut writer = RotatingLogWriter::new(&log).unwrap();

        // Seed file beyond the rotation threshold.
        // Use write mode, not append mode: append ignores seek position.
        {
            let mut seed = std::fs::OpenOptions::new().write(true).open(&log).unwrap();
            seed.seek(SeekFrom::Start(MAX_LOG_SIZE + 1)).unwrap();
            seed.write_all(b"A").unwrap();
            seed.flush().unwrap();
        }

        // Force a rotation check with a big enough write batch.
        writer.write_chunk(&vec![b'x'; ROTATION_CHECK_INTERVAL_BYTES as usize]);
        writer.write_chunk(b"AFTER_ROTATION\n");

        assert!(log1.exists(), "rotated file should exist");
        assert!(log.exists(), "active log should be reopened");

        let active = std::fs::read_to_string(&log).unwrap_or_default();
        assert!(
            active.contains("AFTER_ROTATION"),
            "new writes must go to reopened active log"
        );

        cleanup(&[&log, &log1]);
    }
}

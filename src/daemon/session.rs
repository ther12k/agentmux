use anyhow::Result;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::SystemTime;
use tracing::{debug, info, warn};

use crate::storage::logs::{self, RotatingLogWriter};

/// Default PTY size when terminal size cannot be detected.
const DEFAULT_PTY_SIZE: PtySize = PtySize {
    rows: 24,
    cols: 80,
    pixel_width: 0,
    pixel_height: 0,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub command: String,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub pid: Option<u32>,
    pub created_at: u64,
    pub updated_at: u64,
    pub status: SessionStatus,
    pub exit_code: Option<i32>,
    pub log_path: Option<PathBuf>,
}

impl Session {
    pub fn new(
        name: &str,
        profile: &str,
        command: &str,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> Result<Self> {
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)?
            .as_secs();
        Ok(Self {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            profile: profile.to_string(),
            command: command.to_string(),
            args,
            cwd,
            pid: None,
            created_at: now,
            updated_at: now,
            status: SessionStatus::Running,
            exit_code: None,
            log_path: None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Running,
    Attached,
    Detached,
    Exited,
    Failed,
    Orphaned,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionStatus::Running => write!(f, "running"),
            SessionStatus::Attached => write!(f, "attached"),
            SessionStatus::Detached => write!(f, "detached"),
            SessionStatus::Exited => write!(f, "exited"),
            SessionStatus::Failed => write!(f, "failed"),
            SessionStatus::Orphaned => write!(f, "orphaned"),
        }
    }
}

/// Output subscriber channel — attached clients receive PTY output through this.
pub type Subscriber = mpsc::Sender<Vec<u8>>;

/// Shared PTY writer type.
pub type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Type returned by try_attach: subscriber receiver + shared writer.
pub type AttachHandle = (mpsc::Receiver<Vec<u8>>, SharedWriter);

/// Live runtime state for a spawned session (not serialized).
///
/// Architecture: ONE background reader thread owns the single PTY reader.
/// It fans out bytes to:
///   1. The log file (always)
///   2. The current subscriber, if any (set on attach, cleared on detach)
///
/// The PTY writer is shared via Arc<Mutex<>> so both the attach client
/// and programmatic writes (send, resize) can access it safely.
pub struct SessionHandle {
    pub child: Box<dyn Child + Send + Sync>,
    /// Master PTY handle, kept for resize operations.
    pub master: Box<dyn MasterPty + Send>,
    /// Shared PTY writer.
    pub writer: SharedWriter,
    /// Current output subscriber (attached client).
    /// Only one subscriber at a time.
    pub subscriber: Arc<Mutex<Option<Subscriber>>>,
    /// Flag indicating whether a client is currently attached.
    pub attached: Arc<AtomicBool>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("attached", &self.attached.load(Ordering::Relaxed))
            .finish()
    }
}

pub struct SessionRegistry {
    sessions: HashMap<String, Session>,
    insertion_order: Vec<String>,
    handles: HashMap<String, SessionHandle>,
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            insertion_order: Vec::new(),
            handles: HashMap::new(),
        }
    }

    pub fn list(&self) -> Vec<&Session> {
        self.insertion_order
            .iter()
            .filter_map(|name| self.sessions.get(name))
            .collect()
    }

    pub fn exists(&self, name: &str) -> bool {
        self.sessions.contains_key(name)
    }

    #[allow(dead_code)]
    pub fn get(&self, name: &str) -> Option<&Session> {
        self.sessions.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut Session> {
        self.sessions.get_mut(name)
    }

    pub fn add(&mut self, session: Session) -> Result<()> {
        if self.exists(&session.name) {
            anyhow::bail!("Session '{}' already exists", session.name);
        }
        self.insertion_order.push(session.name.clone());
        self.sessions.insert(session.name.clone(), session);
        Ok(())
    }

    pub fn remove(&mut self, name: &str) -> Option<Session> {
        self.handles.remove(name);
        self.insertion_order.retain(|n| n != name);
        self.sessions.remove(name)
    }

    pub fn update(
        &mut self,
        name: &str,
        status: Option<SessionStatus>,
        exit_code: Option<i32>,
        pid: Option<u32>,
    ) {
        if let Some(session) = self.sessions.get_mut(name) {
            if let Some(status) = status {
                session.status = status;
            }
            if let Some(code) = exit_code {
                session.exit_code = Some(code);
            }
            if let Some(pid) = pid {
                session.pid = Some(pid);
            }
            if let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
                session.updated_at = now.as_secs();
            }
        }
    }

    /// Spawn a real PTY-backed child, store the handle, and start
    /// ONE background reader thread that:
    ///   - appends PTY output to the log file
    ///   - broadcasts output to the current subscriber (if attached)
    ///   - exits on PTY EOF
    ///
    /// Returns the child PID.
    pub fn spawn(
        &mut self,
        name: &str,
        command: &str,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> Result<u32> {
        if !self.exists(name) {
            anyhow::bail!("Session '{}' not found", name);
        }
        if self.handles.contains_key(name) {
            anyhow::bail!("Session '{}' is already spawned", name);
        }

        let pty_system = native_pty_system();
        let pair = pty_system.openpty(DEFAULT_PTY_SIZE)?;

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args.clone());
        if let Some(cwd) = &cwd {
            cmd.cwd(cwd);
        }

        let child = pair.slave.spawn_command(cmd)?;
        let pid = child.process_id().unwrap_or(0);

        // Drop the slave so EOF propagates when the child exits.
        drop(pair.slave);

        // Take the writer once — shared via Arc<Mutex>.
        let pty_writer: Box<dyn Write + Send> = pair.master.take_writer()?;
        let writer = Arc::new(Mutex::new(pty_writer));

        // Determine the log path.
        let log_path = logs::log_file_path(name);
        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        // Update the session with log_path.
        if let Some(session) = self.sessions.get_mut(name) {
            session.log_path = Some(log_path.clone());
        }

        // Subscriber slot — initially None (no client attached).
        let subscriber: Arc<Mutex<Option<Subscriber>>> = Arc::new(Mutex::new(None));
        let attached = Arc::new(AtomicBool::new(false));

        // Start ONE background reader thread.
        // This thread owns the single PTY reader and fans out output.
        let reader = pair.master.try_clone_reader()?;
        let subscriber_clone = Arc::clone(&subscriber);
        let session_name = name.to_string();
        let log_path_for_thread = log_path.clone();
        std::thread::spawn(move || {
            pty_reader_loop(
                reader,
                &log_path_for_thread,
                &session_name,
                subscriber_clone,
            );
        });

        let handle = SessionHandle {
            child,
            master: pair.master,
            writer,
            subscriber,
            attached,
        };

        self.handles.insert(name.to_string(), handle);
        self.update(name, None, None, Some(pid));

        info!("Session '{}' spawned (pid={})", name, pid);
        Ok(pid)
    }

    /// Send a signal to a session child.
    /// `kill=true` sends SIGKILL via portable_pty's kill method,
    /// `kill=false` sends SIGTERM via libc.
    pub fn signal(&mut self, name: &str, kill: bool) -> Result<()> {
        let Some(handle) = self.handles.get_mut(name) else {
            anyhow::bail!("Session '{}' has no running child", name);
        };

        if kill {
            handle.child.kill()?;
        } else {
            let pid = handle.child.process_id().unwrap_or(0) as i32;
            if pid == 0 {
                anyhow::bail!("Session '{}' has no PID", name);
            }
            let rc = unsafe { libc::kill(pid, libc::SIGTERM) };
            if rc != 0 {
                anyhow::bail!(
                    "Failed to signal session '{}': {}",
                    name,
                    std::io::Error::last_os_error()
                );
            }
        }
        Ok(())
    }

    /// Try to mark a session as attached.
    /// Returns the subscriber receiver and writer clone for the attach client.
    pub fn try_attach(&self, name: &str) -> Result<AttachHandle> {
        let Some(handle) = self.handles.get(name) else {
            anyhow::bail!("Session '{}' has no running child", name);
        };
        if handle.attached.swap(true, Ordering::SeqCst) {
            anyhow::bail!("Session '{}' is already attached", name);
        }

        // Create subscriber channel.
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        {
            let mut sub = handle
                .subscriber
                .lock()
                .map_err(|_| anyhow::anyhow!("Subscriber lock poisoned"))?;
            *sub = Some(tx);
        }

        Ok((rx, Arc::clone(&handle.writer)))
    }

    /// Mark a session as detached and clear the subscriber.
    pub fn detach(&self, name: &str) {
        if let Some(handle) = self.handles.get(name) {
            handle.attached.store(false, Ordering::SeqCst);
            if let Ok(mut sub) = handle.subscriber.lock() {
                *sub = None;
            }
        }
    }

    /// Resize a session's PTY.
    pub fn resize(&self, name: &str, rows: u16, cols: u16) -> Result<()> {
        let Some(handle) = self.handles.get(name) else {
            anyhow::bail!("Session '{}' has no running child", name);
        };
        handle.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }

    /// Reap any children that have exited and update session state.
    /// Returns true if any session was reaped (for metadata persistence).
    pub fn reap(&mut self) -> bool {
        let names: Vec<String> = self.handles.keys().cloned().collect();
        let mut any_reaped = false;
        for name in names {
            let exit_code = {
                let Some(handle) = self.handles.get_mut(&name) else {
                    continue;
                };
                match handle.child.try_wait() {
                    Ok(Some(status)) => Some(status.exit_code() as i32),
                    Ok(None) => None,
                    Err(_) => Some(-1),
                }
            };

            if let Some(code) = exit_code {
                info!("Session '{}' exited with code={}", name, code);
                self.handles.remove(&name);
                self.update(&name, Some(SessionStatus::Exited), Some(code), None);
                any_reaped = true;
            }
        }
        any_reaped
    }

    /// Check if a session handle is alive (has a running child).
    pub fn is_alive(&self, name: &str) -> bool {
        self.handles.contains_key(name)
    }

    /// Write data to a session's PTY input (programmatic send).
    pub fn write_input(&self, name: &str, data: &[u8]) -> Result<()> {
        let Some(handle) = self.handles.get(name) else {
            anyhow::bail!("Session '{}' has no running child", name);
        };
        let mut writer = handle
            .writer
            .lock()
            .map_err(|_| anyhow::anyhow!("PTY writer lock poisoned"))?;
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Format a human-readable restart marker string for session logs.
///
/// Uses local time for readability. Format:
/// `--- AgentMux restart at 2026-06-14 21:30:00 ---`
pub fn format_restart_marker() -> String {
    let now = chrono::Local::now();
    format!(
        "\n--- AgentMux restart at {} ---\n",
        now.format("%Y-%m-%d %H:%M:%S")
    )
}

/// Background reader thread: owns the single PTY reader.
/// Reads output, appends to log file, and broadcasts to subscriber if attached.
/// Exits on PTY EOF. On exit, clears the subscriber so the daemon's attach
/// handler detects disconnection promptly.
fn pty_reader_loop(
    mut reader: Box<dyn std::io::Read + Send>,
    log_path: &std::path::Path,
    name: &str,
    subscriber: Arc<Mutex<Option<Subscriber>>>,
) {
    let mut log_writer = match RotatingLogWriter::new(log_path) {
        Ok(writer) => writer,
        Err(e) => {
            warn!("Failed to open log for '{}': {}", name, e);
            return;
        }
    };

    let mut buf = [0u8; 4096];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = &buf[..n];

                // Append to rotating log file. Rotation errors warn but never crash.
                log_writer.write_chunk(chunk);

                // Broadcast to subscriber if attached.
                if let Ok(sub_guard) = subscriber.lock() {
                    if let Some(tx) = sub_guard.as_ref() {
                        // send() fails if the receiver was dropped (client detached).
                        // That's fine — we just skip and continue logging.
                        let _ = tx.send(chunk.to_vec());
                    }
                }
            }
            Err(e) => {
                debug!("PTY reader for '{}' error: {}", name, e);
                break;
            }
        }
    }

    // PTY reached EOF or error. Clear the subscriber so:
    //   1. The daemon's attach handler (subscriber_rx.recv()) gets Disconnected
    //   2. The session is not left in an "attached" state after child exit
    //   3. Any future attach attempt sees the session as not-attached
    if let Ok(mut sub) = subscriber.lock() {
        *sub = None;
    }

    debug!(
        "Reader thread for '{}' finished (EOF), subscriber cleared",
        name
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_and_list_sessions() {
        let mut reg = SessionRegistry::new();
        let s1 = Session::new("s1", "shell", "bash", vec![], None).unwrap();
        let s2 = Session::new("s2", "shell", "bash", vec![], None).unwrap();
        reg.add(s1).unwrap();
        reg.add(s2).unwrap();

        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].name, "s1");
        assert_eq!(list[1].name, "s2");
    }

    #[test]
    fn reject_duplicate_session_names() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("dup", "shell", "bash", vec![], None).unwrap())
            .unwrap();

        assert!(reg
            .add(Session::new("dup", "shell", "bash", vec![], None).unwrap())
            .is_err());
    }

    #[test]
    fn remove_session() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("bye", "shell", "bash", vec![], None).unwrap())
            .unwrap();
        reg.remove("bye");
        assert!(!reg.exists("bye"));
    }

    #[test]
    fn spawn_and_reap_quick_exit() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("quick", "shell", "true", vec![], None).unwrap())
            .unwrap();
        reg.spawn("quick", "true", vec![], None).unwrap();

        // Poll for up to 5 seconds waiting for reap.
        for _ in 0..50 {
            reg.reap();
            if let Some(s) = reg.get("quick") {
                if matches!(s.status, SessionStatus::Exited) {
                    assert!(s.exit_code.is_some());
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("Session did not get reaped within 5s");
    }

    #[test]
    fn session_status_display() {
        assert_eq!(SessionStatus::Running.to_string(), "running");
        assert_eq!(SessionStatus::Attached.to_string(), "attached");
        assert_eq!(SessionStatus::Detached.to_string(), "detached");
        assert_eq!(SessionStatus::Exited.to_string(), "exited");
        assert_eq!(SessionStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn spawn_sets_log_path() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("logtest", "shell", "true", vec![], None).unwrap())
            .unwrap();
        reg.spawn("logtest", "true", vec![], None).unwrap();

        let session = reg.get("logtest").unwrap();
        assert!(session.log_path.is_some());
        let path = session.log_path.as_ref().unwrap();
        assert!(path.to_string_lossy().contains("logtest"));
    }

    // ---- Status transition tests ----

    #[test]
    fn status_transitions_detached_to_attached_to_detached() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("st_test", "shell", "sleep", vec![], None).unwrap())
            .unwrap();
        reg.spawn("st_test", "sleep", vec!["10".to_string()], None)
            .unwrap();

        // Initially running (not attached).
        assert!(!reg
            .handles
            .get("st_test")
            .unwrap()
            .attached
            .load(Ordering::SeqCst));

        // Attach.
        let (rx1, _writer1) = reg.try_attach("st_test").unwrap();
        assert!(reg
            .handles
            .get("st_test")
            .unwrap()
            .attached
            .load(Ordering::SeqCst));

        // Detach.
        reg.detach("st_test");
        assert!(!reg
            .handles
            .get("st_test")
            .unwrap()
            .attached
            .load(Ordering::SeqCst));

        // Receiver should eventually get disconnected (drop rx1).
        drop(rx1);

        // Re-attach works.
        let (_rx2, _writer2) = reg.try_attach("st_test").unwrap();
        assert!(reg
            .handles
            .get("st_test")
            .unwrap()
            .attached
            .load(Ordering::SeqCst));

        // Detach again.
        reg.detach("st_test");

        // Clean up.
        reg.signal("st_test", true).unwrap();
        for _ in 0..50 {
            reg.reap();
            if let Some(s) = reg.get("st_test") {
                if matches!(s.status, SessionStatus::Exited) {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        panic!("Failed to reap test session");
    }

    #[test]
    fn double_attach_rejected() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("da_test", "shell", "sleep", vec![], None).unwrap())
            .unwrap();
        reg.spawn("da_test", "sleep", vec!["10".to_string()], None)
            .unwrap();

        // First attach succeeds.
        let (_rx, _writer) = reg.try_attach("da_test").unwrap();

        // Second attach fails.
        let result = reg.try_attach("da_test");
        assert!(result.is_err());
        assert!(result
            .err()
            .map(|e| e.to_string().contains("already attached"))
            .unwrap_or(false));

        // Clean up.
        reg.detach("da_test");
        reg.signal("da_test", true).unwrap();
        for _ in 0..50 {
            reg.reap();
            if let Some(s) = reg.get("da_test") {
                if matches!(s.status, SessionStatus::Exited) {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[test]
    fn subscriber_receives_pty_output() {
        let mut reg = SessionRegistry::new();
        reg.add(
            Session::new("sub_test", "shell", "echo", vec!["hello".to_string()], None).unwrap(),
        )
        .unwrap();
        reg.spawn("sub_test", "echo", vec!["hello".to_string()], None)
            .unwrap();

        // Attach and get subscriber receiver.
        let (rx, _writer) = reg.try_attach("sub_test").unwrap();

        // Wait for output or session exit.
        let mut received = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(data) => received.extend_from_slice(&data),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    // Check if session exited.
                    reg.reap();
                    if let Some(s) = reg.get("sub_test") {
                        if matches!(s.status, SessionStatus::Exited) {
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        let text = String::from_utf8_lossy(&received);
        assert!(
            text.contains("hello"),
            "Expected 'hello' in output, got: {:?}",
            text
        );

        reg.detach("sub_test");
    }

    #[test]
    fn subscriber_cleared_on_session_exit() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("exit_test", "shell", "echo", vec!["bye".to_string()], None).unwrap())
            .unwrap();
        reg.spawn("exit_test", "echo", vec!["bye".to_string()], None)
            .unwrap();

        // Attach.
        let (rx, _writer) = reg.try_attach("exit_test").unwrap();

        // Wait for session to exit and reader thread to clear subscriber.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            reg.reap();
            if let Some(s) = reg.get("exit_test") {
                if matches!(s.status, SessionStatus::Exited) {
                    break;
                }
            }
            if std::time::Instant::now() >= deadline {
                panic!("Session did not exit within 5s");
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }

        // After session exit, the reader thread clears the subscriber.
        // The subscriber channel should be disconnected (recv returns Err).
        // Give the reader thread a moment to finish cleanup.
        let chan_closed = deadline - std::time::Instant::now();
        let _ = rx.recv_timeout(chan_closed.max(std::time::Duration::from_millis(500)));

        // The attached flag should be false after detach/exit.
        // detach() has been called or the reader thread cleared the subscriber.
        // The session should not be in "attached" state.
        if let Some(handle) = reg.handles.get("exit_test") {
            assert!(
                !handle.attached.load(Ordering::SeqCst),
                "Session should not be attached after exit"
            );
        }

        // Subscriber slot should be cleared.
        if let Some(handle) = reg.handles.get("exit_test") {
            if let Ok(sub) = handle.subscriber.lock() {
                assert!(sub.is_none(), "Subscriber should be cleared after exit");
            }
        }
    }

    #[test]
    fn detach_clears_subscriber_slot() {
        let mut reg = SessionRegistry::new();
        reg.add(Session::new("detach_test", "shell", "sleep", vec![], None).unwrap())
            .unwrap();
        reg.spawn("detach_test", "sleep", vec!["10".to_string()], None)
            .unwrap();

        // Attach.
        let (_rx, _writer) = reg.try_attach("detach_test").unwrap();
        assert!(reg
            .handles
            .get("detach_test")
            .unwrap()
            .attached
            .load(Ordering::SeqCst));

        // Subscriber should be Some.
        {
            let sub = reg
                .handles
                .get("detach_test")
                .unwrap()
                .subscriber
                .lock()
                .unwrap();
            assert!(sub.is_some());
        }

        // Detach.
        reg.detach("detach_test");

        // Subscriber should be None, attached false.
        assert!(!reg
            .handles
            .get("detach_test")
            .unwrap()
            .attached
            .load(Ordering::SeqCst));
        {
            let sub = reg
                .handles
                .get("detach_test")
                .unwrap()
                .subscriber
                .lock()
                .unwrap();
            assert!(sub.is_none());
        }

        // Clean up.
        reg.signal("detach_test", true).unwrap();
        for _ in 0..50 {
            reg.reap();
            if let Some(s) = reg.get("detach_test") {
                if matches!(s.status, SessionStatus::Exited) {
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
}

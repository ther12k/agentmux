use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use tracing::{debug, error, info, warn};

use super::metadata;
use super::protocol::{Request, Response};
use super::session::{Session, SessionRegistry, SessionStatus};
use super::state;

/// Type alias for the shared registry.
type SharedRegistry = Arc<Mutex<SessionRegistry>>;

/// Default timeout for graceful shutdown (seconds).
const DEFAULT_SHUTDOWN_TIMEOUT_SECS: u64 = 5;

/// Start the daemon: open socket, spawn reaper thread, handle connections.
///
/// Architecture:
///   - Startup: load sessions.json for recovery
///   - Main thread: accept loop, spawns a thread per connection
///   - Reaper thread: periodically locks registry and reaps exited children
///   - Per-connection thread: handles request/response, or raw attach forwarding
pub fn run() -> Result<()> {
    let sock_path = state::socket_path()?;

    // Remove stale socket
    if sock_path.exists() {
        if state::is_daemon_running() {
            anyhow::bail!(
                "Daemon is already running (socket active: {})",
                sock_path.display()
            );
        }
        std::fs::remove_file(&sock_path)?;
    }

    let listener = UnixListener::bind(&sock_path)?;
    info!("agentmux daemon listening on {}", sock_path.display());

    // Set socket permissions
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(&sock_path, perms)?;

    // Write PID file for doctor diagnostics.
    state::write_pid_file();

    let registry: SharedRegistry = Arc::new(Mutex::new(SessionRegistry::new()));

    // Recover sessions from sessions.json
    recover_from_metadata(&registry);

    // Track daemon start time for uptime.
    let start_time = Instant::now();

    // Dedicated reaper thread: locks registry directly every 500ms.
    // Also persists metadata periodically.
    {
        let registry = Arc::clone(&registry);
        thread::spawn(move || loop {
            thread::sleep(Duration::from_millis(500));
            if let Ok(mut reg) = registry.lock() {
                let changed = reg.reap();
                if changed {
                    drop(reg);
                    if let Ok(reg) = registry.lock() {
                        let _ = metadata::save_metadata(&reg);
                    }
                }
            }
        });
    }

    // Use a timeout on accept so we can check a shutdown flag.
    listener.set_nonblocking(true)?;

    let shutting_down = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // Accept loop: spawn a thread per connection.
    loop {
        // Check if shutdown was requested.
        if shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }

        match listener.accept() {
            Ok((stream, _)) => {
                // Reset to blocking for the connection handler.
                stream.set_nonblocking(false)?;
                let registry = Arc::clone(&registry);
                let shutting_down = Arc::clone(&shutting_down);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, &registry, &shutting_down, start_time)
                    {
                        error!("Connection error: {}", e);
                    }
                });
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                // No connection ready, brief sleep and retry.
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => {
                error!("Accept error: {}", e);
            }
        }
    }

    // Graceful shutdown: cleanup already done by do_shutdown.
    info!("Daemon accept loop exited");
    Ok(())
}

/// Recover sessions from sessions.json on startup.
fn recover_from_metadata(registry: &SharedRegistry) {
    match metadata::recover_sessions() {
        Ok(recovered) => {
            if recovered.is_empty() {
                return;
            }
            info!("Recovering {} session(s) from metadata", recovered.len());
            if let Ok(mut reg) = registry.lock() {
                for meta in recovered {
                    let status = match meta.status.as_str() {
                        "orphaned" => SessionStatus::Orphaned,
                        "exited" => SessionStatus::Exited,
                        "running" => SessionStatus::Orphaned,
                        _ => SessionStatus::Exited,
                    };
                    let session = Session {
                        id: meta.id,
                        name: meta.name.clone(),
                        profile: meta.profile,
                        command: meta.command,
                        args: meta.args,
                        cwd: meta.cwd,
                        pid: meta.pid,
                        created_at: meta.created_at,
                        updated_at: meta.updated_at,
                        status,
                        exit_code: meta.exit_code,
                        log_path: meta.log_path.map(std::path::PathBuf::from),
                    };
                    if let Err(e) = reg.add(session) {
                        warn!("Failed to recover session '{}': {}", meta.name, e);
                    }
                }
            }
            // Persist the updated statuses.
            if let Ok(reg) = registry.lock() {
                let _ = metadata::save_metadata(&reg);
            }
        }
        Err(e) => {
            warn!("Failed to load session metadata: {}", e);
        }
    }
}

/// Perform graceful shutdown.
/// 1. Detach all active clients
/// 2. SIGTERM all child sessions
/// 3. Wait up to timeout
/// 4. SIGKILL remaining
/// 5. Update statuses
/// 6. Persist metadata
/// 7. Remove socket + PID file
fn do_shutdown(registry: &SharedRegistry) -> Response {
    info!("Graceful shutdown initiated");

    // SIGTERM all running sessions.
    {
        if let Ok(reg) = registry.lock() {
            let names: Vec<String> = reg.list().iter().map(|s| s.name.clone()).collect();
            drop(reg);
            for name in names {
                if let Ok(mut reg) = registry.lock() {
                    if reg.is_alive(&name) {
                        let _ = reg.signal(&name, false); // SIGTERM
                        debug!("SIGTERM sent to '{}'", name);
                    }
                }
            }
        }
    }

    // Wait up to DEFAULT_SHUTDOWN_TIMEOUT_SECS for children to exit.
    let deadline = Instant::now() + Duration::from_secs(DEFAULT_SHUTDOWN_TIMEOUT_SECS);
    loop {
        thread::sleep(Duration::from_millis(200));
        if let Ok(mut reg) = registry.lock() {
            reg.reap();
        }
        if Instant::now() >= deadline {
            break;
        }
        // Check if all done.
        if let Ok(reg) = registry.lock() {
            let any_alive = reg.list().iter().any(|s| {
                matches!(
                    s.status,
                    SessionStatus::Running | SessionStatus::Attached | SessionStatus::Detached
                )
            });
            if !any_alive {
                break;
            }
        }
    }

    // SIGKILL any remaining live children. Do not overwrite already-reaped
    // sessions, because they may have a real exit code from the graceful wait.
    {
        if let Ok(reg) = registry.lock() {
            let names: Vec<String> = reg.list().iter().map(|s| s.name.clone()).collect();
            drop(reg);
            for name in names {
                if let Ok(mut reg) = registry.lock() {
                    if reg.is_alive(&name) {
                        let _ = reg.signal(&name, true); // SIGKILL
                        debug!("SIGKILL sent to '{}'", name);
                        reg.update(&name, Some(SessionStatus::Exited), Some(-1), None);
                    }
                }
            }
        }
    }

    // Persist metadata.
    if let Ok(reg) = registry.lock() {
        let _ = metadata::save_metadata(&reg);
    }

    // Remove socket and PID file. We own the listener during shutdown, so do
    // not call remove_stale_socket(): it intentionally refuses to remove a
    // socket while the daemon is still responsive.
    if let Ok(sock_path) = state::socket_path() {
        if sock_path.exists() {
            let _ = std::fs::remove_file(&sock_path);
        }
    }
    state::remove_pid_file();

    info!("Shutdown complete");
    Response::ok(serde_json::json!({"status": "shutting down"}))
}

/// Handle a single client connection.
/// For most requests: read one JSON line, respond with one JSON line.
/// For AttachSession: after responding, switch to raw byte forwarding.
fn handle_connection(
    stream: UnixStream,
    registry: &SharedRegistry,
    shutting_down: &Arc<std::sync::atomic::AtomicBool>,
    start_time: Instant,
) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut writer = stream;

    let mut line = String::new();
    reader.read_line(&mut line)?;

    let request: Request = match serde_json::from_str(&line) {
        Ok(r) => r,
        Err(e) => {
            let resp = Response::error(&format!("Invalid request: {}", e));
            let json = serde_json::to_string(&resp)?;
            writer.write_all(format!("{}\n", json).as_bytes())?;
            return Ok(());
        }
    };

    info!("Request: {:?}", request);

    // Check if this is an attach request — handle specially.
    if let Request::AttachSession { ref name } = request {
        return handle_attach(writer, name, registry);
    }

    if let Request::DetachSession { ref name } = request {
        if let Ok(reg) = registry.lock() {
            reg.detach(name);
        }
        let resp = Response::ok(serde_json::json!({ "status": "detached" }));
        let json = serde_json::to_string(&resp)?;
        writer.write_all(format!("{}\n", json).as_bytes())?;
        return Ok(());
    }

    // Handle Shutdown specially — sets flag and does graceful cleanup.
    if matches!(request, Request::Shutdown) {
        let resp = do_shutdown(registry);
        let json = serde_json::to_string(&resp)?;
        writer.write_all(format!("{}\n", json).as_bytes())?;
        // Signal the accept loop to stop.
        shutting_down.store(true, std::sync::atomic::Ordering::SeqCst);
        // Exit the daemon process.
        std::process::exit(0);
    }

    let response = handle_request(request, registry, start_time)?;

    let json = serde_json::to_string(&response)?;
    writer.write_all(format!("{}\n", json).as_bytes())?;

    Ok(())
}

/// Handle attach request:
/// 1. Register a subscriber channel with the session
/// 2. Send OK response
/// 3. Spawn threads to bridge socket↔subscriber and socket→PTY writer
/// 4. On client disconnect (Ctrl-b d) or PTY exit, clean up and return
fn handle_attach(mut stream: UnixStream, name: &str, registry: &SharedRegistry) -> Result<()> {
    // Try to attach.
    let (subscriber_rx, pty_writer) = {
        let reg = registry
            .lock()
            .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

        // Check the session exists.
        if !reg.exists(name) {
            let resp = Response::error(&format!("Session '{}' not found", name));
            let json = serde_json::to_string(&resp)?;
            stream.write_all(format!("{}\n", json).as_bytes())?;
            return Ok(());
        }

        // Check if orphaned.
        if let Some(session) = reg.get(name) {
            if matches!(session.status, SessionStatus::Orphaned) {
                let resp = Response::error(
                    "session is orphaned after daemon restart; restart it to attach again",
                );
                let json = serde_json::to_string(&resp)?;
                stream.write_all(format!("{}\n", json).as_bytes())?;
                return Ok(());
            }
        }

        // Check the session is alive (has a PTY handle).
        if !reg.is_alive(name) {
            let resp = Response::error(&format!("Session '{}' not found or not running", name));
            let json = serde_json::to_string(&resp)?;
            stream.write_all(format!("{}\n", json).as_bytes())?;
            return Ok(());
        }

        match reg.try_attach(name) {
            Ok((rx, writer)) => (rx, writer),
            Err(e) => {
                let resp = Response::error(&e.to_string());
                let json = serde_json::to_string(&resp)?;
                stream.write_all(format!("{}\n", json).as_bytes())?;
                return Ok(());
            }
        }
    };

    // Update session status to Attached.
    {
        if let Ok(mut reg) = registry.lock() {
            reg.update(name, Some(SessionStatus::Attached), None, None);
        }
    }

    // Send success response.
    let resp = Response::ok(serde_json::json!({ "status": "attached" }));
    let json = serde_json::to_string(&resp)?;
    stream.write_all(format!("{}\n", json).as_bytes())?;
    stream.flush()?;

    // Now switch to raw byte forwarding mode.
    let write_stream = stream.try_clone()?;
    let shutdown_stream = stream.try_clone()?;

    let (exit_tx, exit_rx) = mpsc::channel::<()>();
    let exit_tx_pty = exit_tx.clone();
    let exit_tx_sock = exit_tx.clone();
    drop(exit_tx);

    // Thread: subscriber → socket (PTY output to client)
    let pty_to_socket_thread = thread::spawn(move || {
        let mut stream = write_stream;
        loop {
            match subscriber_rx.recv() {
                Ok(data) => {
                    if stream.write_all(&data).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                }
                Err(mpsc::RecvError) => {
                    debug!("Subscriber channel closed (session may have exited)");
                    break;
                }
            }
        }
        let _ = exit_tx_pty.send(());
    });

    // Thread: socket → PTY writer (client input to PTY)
    let registry_for_cleanup = Arc::clone(registry);
    let name_owned = name.to_string();
    let socket_to_pty_thread = thread::spawn(move || {
        let mut stream_reader = stream;
        let mut buf = [0u8; 4096];
        loop {
            match stream_reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let mut writer = match pty_writer.lock() {
                        Ok(w) => w,
                        Err(_) => break,
                    };
                    if writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                    let _ = writer.flush();
                }
                Err(_) => break,
            }
        }

        if let Ok(reg) = registry_for_cleanup.lock() {
            reg.detach(&name_owned);
        }
        if let Ok(mut reg) = registry_for_cleanup.lock() {
            if reg.is_alive(&name_owned) {
                reg.update(&name_owned, Some(SessionStatus::Detached), None, None);
            }
        }

        let _ = exit_tx_sock.send(());
    });

    let _ = exit_rx.recv();
    let _ = shutdown_stream.shutdown(std::net::Shutdown::Both);
    let _ = pty_to_socket_thread.join();
    let _ = socket_to_pty_thread.join();

    // Final cleanup.
    if let Ok(reg) = registry.lock() {
        reg.detach(name);
    }
    if let Ok(mut reg) = registry.lock() {
        if reg.is_alive(name) {
            reg.update(name, Some(SessionStatus::Detached), None, None);
        }
    }

    Ok(())
}

fn handle_request(
    request: Request,
    registry: &SharedRegistry,
    start_time: Instant,
) -> Result<Response> {
    Ok(match request {
        Request::Ping => Response::ok(serde_json::json!("pong")),

        Request::DaemonStatus => {
            let pid = std::process::id();
            let uptime = start_time.elapsed().as_secs();
            let session_count = {
                let reg = registry
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
                reg.list().len()
            };
            let sock_path = state::socket_path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            Response::ok(serde_json::json!({
                "pid": pid,
                "uptime_seconds": uptime,
                "session_count": session_count,
                "socket_path": sock_path,
            }))
        }

        Request::ListSessions => {
            let sessions = {
                let mut reg = registry
                    .lock()
                    .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
                reg.reap();
                serde_json::to_value(reg.list())?
            };
            Response::ok(sessions)
        }

        Request::AddSession {
            name,
            profile,
            command,
            args,
            cwd,
        } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
            match Session::new(&name, &profile, &command, args, cwd) {
                Ok(session) => match reg.add(session) {
                    Ok(_) => {
                        let _ = metadata::save_metadata(&reg);
                        Response::ok(serde_json::json!({ "status": "created" }))
                    }
                    Err(e) => Response::error(&e.to_string()),
                },
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::RemoveSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
            reg.remove(&name);
            let _ = metadata::save_metadata(&reg);
            Response::ok(serde_json::json!({ "status": "removed" }))
        }

        Request::SpawnSession {
            name,
            profile,
            command,
            args,
            cwd,
        } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            // Auto-create the session entry if it doesn't exist yet. If a
            // previous exited/failed entry exists, allow respawn and refresh its
            // metadata in SessionRegistry::spawn. Do not spawn over a live or
            // orphaned session, because that can duplicate agents with one name.
            if reg.exists(&name) {
                if reg.is_alive(&name) {
                    return Ok(Response::error(&format!(
                        "Session '{}' is already running",
                        name
                    )));
                }
                if let Some(existing) = reg.get(&name) {
                    if matches!(existing.status, SessionStatus::Orphaned) {
                        return Ok(Response::error(
                            "session is orphaned after daemon restart; restart it first",
                        ));
                    }
                }
            } else {
                let session = Session::new(&name, &profile, &command, args.clone(), cwd.clone())?;
                reg.add(session)?;
            }

            match reg.spawn(&name, &command, args, cwd) {
                Ok(pid) => {
                    let _ = metadata::save_metadata(&reg);
                    Response::ok(serde_json::json!({
                        "status": "spawned",
                        "pid": pid,
                    }))
                }
                Err(e) => {
                    reg.update(&name, Some(SessionStatus::Failed), None, None);
                    let _ = metadata::save_metadata(&reg);
                    Response::error(&e.to_string())
                }
            }
        }

        Request::StopSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            // Handle orphaned sessions: try to kill PID.
            if let Some(session) = reg.get(&name).cloned() {
                if matches!(session.status, SessionStatus::Orphaned) {
                    if let Some(pid) = session.pid {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGTERM);
                        }
                    }
                    reg.update(&name, Some(SessionStatus::Exited), None, None);
                    let _ = metadata::save_metadata(&reg);
                    return Ok(Response::ok(serde_json::json!({ "status": "stopped" })));
                }
            }

            match reg.signal(&name, false) {
                Ok(_) => {
                    let _ = metadata::save_metadata(&reg);
                    Response::ok(serde_json::json!({ "status": "stopped" }))
                }
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::KillSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            // Handle orphaned sessions: try to kill PID.
            if let Some(session) = reg.get(&name).cloned() {
                if matches!(session.status, SessionStatus::Orphaned) {
                    if let Some(pid) = session.pid {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                    }
                    reg.update(&name, Some(SessionStatus::Exited), Some(-1), None);
                    let _ = metadata::save_metadata(&reg);
                    return Ok(Response::ok(serde_json::json!({ "status": "killed" })));
                }
            }

            match reg.signal(&name, true) {
                Ok(_) => {
                    let _ = metadata::save_metadata(&reg);
                    Response::ok(serde_json::json!({ "status": "killed" }))
                }
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::ResizeSession { name, rows, cols } => {
            let reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            // Check orphaned.
            if let Some(session) = reg.get(&name) {
                if matches!(session.status, SessionStatus::Orphaned) {
                    return Ok(Response::error("session is orphaned; restart it to attach"));
                }
            }

            match reg.resize(&name, rows, cols) {
                Ok(_) => Response::ok(serde_json::json!({ "status": "resized" })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::SendInput { name, data } => {
            let reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            // Check orphaned.
            if let Some(session) = reg.get(&name) {
                if matches!(session.status, SessionStatus::Orphaned) {
                    return Ok(Response::error("session is orphaned; restart it first"));
                }
            }

            match reg.write_input(&name, &data) {
                Ok(_) => Response::ok(serde_json::json!({ "status": "sent" })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::RestartSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            let session = match reg.get(&name) {
                Some(s) => s.clone(),
                None => return Ok(Response::error(&format!("Session '{}' not found", name))),
            };

            // Write restart marker to log.
            if let Some(log_path) = &session.log_path {
                let marker = crate::daemon::session::format_restart_marker();
                let _ = std::fs::OpenOptions::new()
                    .append(true)
                    .open(log_path)
                    .and_then(|mut f| std::io::Write::write_all(&mut f, marker.as_bytes()));
            }

            // For orphaned sessions: kill old PID if alive.
            // For regular sessions: stop, wait, force kill.
            if matches!(session.status, SessionStatus::Orphaned) {
                if let Some(pid) = session.pid {
                    if state::is_pid_alive(pid) {
                        unsafe {
                            libc::kill(pid as i32, libc::SIGKILL);
                        }
                        thread::sleep(Duration::from_millis(200));
                    }
                }
                reg.remove(&name);
            } else {
                // Stop if alive.
                if reg.is_alive(&name) {
                    let _ = reg.signal(&name, false); // SIGTERM
                }

                // Wait up to 3 seconds for graceful exit.
                for _ in 0..30 {
                    reg.reap();
                    if !reg.is_alive(&name) {
                        break;
                    }
                    thread::sleep(Duration::from_millis(100));
                }

                // Force kill if still alive.
                if reg.is_alive(&name) {
                    let _ = reg.signal(&name, true);
                    thread::sleep(Duration::from_millis(200));
                    reg.reap();
                }

                reg.remove(&name);
            }

            // Create new session with same metadata and spawn.
            let new_session = Session::new(
                &session.name,
                &session.profile,
                &session.command,
                session.args.clone(),
                session.cwd.clone(),
            )?;
            reg.add(new_session)?;

            match reg.spawn(
                &name,
                &session.command,
                session.args.clone(),
                session.cwd.clone(),
            ) {
                Ok(pid) => {
                    let _ = metadata::save_metadata(&reg);
                    Response::ok(serde_json::json!({ "status": "restarted", "pid": pid }))
                }
                Err(e) => {
                    reg.update(&name, Some(SessionStatus::Failed), None, None);
                    let _ = metadata::save_metadata(&reg);
                    Response::error(&e.to_string())
                }
            }
        }

        Request::AttachSession { .. } | Request::DetachSession { .. } => {
            Response::error("Attach/Detach should not reach handle_request")
        }

        Request::Shutdown => {
            // This is handled in handle_connection before reaching here.
            Response::error("Shutdown should not reach handle_request")
        }
    })
}

/// Send a request to the daemon and get a response.
pub fn send_request(request: &Request) -> Result<Response> {
    let sock_path = state::socket_path()?;

    if !sock_path.exists() {
        return Err(anyhow::anyhow!(
            "Daemon is not running (socket not found: {})",
            sock_path.display()
        ));
    }

    let mut stream = UnixStream::connect(&sock_path)?;

    let json = serde_json::to_string(request)?;
    stream.write_all(format!("{}\n", json).as_bytes())?;

    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let response: Response = serde_json::from_str(&line)?;
    Ok(response)
}

/// Send a resize request to the daemon (short-lived connection).
pub fn send_resize(name: &str, rows: u16, cols: u16) -> Result<()> {
    let request = Request::ResizeSession {
        name: name.to_string(),
        rows,
        cols,
    };
    let resp = send_request(&request)?;
    if !resp.ok {
        warn!("Resize failed: {:?}", resp.data);
    }
    Ok(())
}

/// Connect to the daemon for an attach session.
/// Returns the connected stream (after reading the response line).
pub fn connect_attach(name: &str) -> Result<UnixStream> {
    let sock_path = state::socket_path()?;

    if !sock_path.exists() {
        return Err(anyhow::anyhow!(
            "Daemon is not running (socket not found: {})",
            sock_path.display()
        ));
    }

    let mut stream = UnixStream::connect(&sock_path)?;

    let request = Request::AttachSession {
        name: name.to_string(),
    };
    let json = serde_json::to_string(&request)?;
    stream.write_all(format!("{}\n", json).as_bytes())?;

    // Read response line.
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;

    let response: Response = serde_json::from_str(&line)?;
    if !response.ok {
        let err = response
            .data
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("attach failed");
        anyhow::bail!("{}", err);
    }

    Ok(stream)
}

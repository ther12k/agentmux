use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;

use anyhow::Result;
use tracing::{debug, error, info, warn};

use super::protocol::{Request, Response};
use super::session::{Session, SessionRegistry, SessionStatus};
use super::state;

/// Type alias for the shared registry.
type SharedRegistry = Arc<Mutex<SessionRegistry>>;

/// Start the daemon: open socket, spawn reaper thread, handle connections.
///
/// Architecture:
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

    // Dedicated reaper thread: locks registry directly every 500ms.
    {
        let registry = Arc::clone(&registry);
        thread::spawn(move || loop {
            thread::sleep(std::time::Duration::from_millis(500));
            if let Ok(mut reg) = registry.lock() {
                reg.reap();
            }
        });
    }

    // Accept loop: spawn a thread per connection.
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let registry = Arc::clone(&registry);
                thread::spawn(move || {
                    if let Err(e) = handle_connection(stream, &registry) {
                        error!("Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                error!("Accept error: {}", e);
            }
        }
    }

    Ok(())
}

/// Handle a single client connection.
/// For most requests: read one JSON line, respond with one JSON line.
/// For AttachSession: after responding, switch to raw byte forwarding.
fn handle_connection(stream: UnixStream, registry: &SharedRegistry) -> Result<()> {
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

    let response = handle_request(request, registry)?;

    let json = serde_json::to_string(&response)?;
    writer.write_all(format!("{}\n", json).as_bytes())?;

    Ok(())
}

/// Handle attach request:
/// 1. Register a subscriber channel with the session
/// 2. Send OK response
/// 3. Spawn threads to bridge socket↔subscriber and socket→PTY writer
/// 4. On client disconnect (Ctrl-b d) or PTY exit, clean up and return
///
/// Uses a channel so the handler returns when EITHER thread exits:
///   - If the client disconnects (Ctrl-b d → socket EOF on read side),
///     socket_to_pty exits and we shut down the subscriber forwarding.
///   - If the PTY reader exits (child process exits → subscriber channel closes),
///     pty_to_socket exits and we shut down the socket forwarding.
///
/// This prevents the handler from hanging when the child process exits while
/// the client is still connected.
fn handle_attach(mut stream: UnixStream, name: &str, registry: &SharedRegistry) -> Result<()> {
    // Try to attach.
    let (subscriber_rx, pty_writer) = {
        let reg = registry
            .lock()
            .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

        // Check the session is alive.
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
    // After the response line, all data is raw bytes.

    let write_stream = stream.try_clone()?;
    // Keep a reference for shutdown after one thread exits.
    let shutdown_stream = stream.try_clone()?;

    // Channel: either forwarding thread signals when it exits.
    let (exit_tx, exit_rx) = mpsc::channel::<()>();
    let exit_tx_pty = exit_tx.clone();
    let exit_tx_sock = exit_tx.clone();
    drop(exit_tx);

    // Thread: subscriber → socket (PTY output to client)
    let pty_to_socket_thread = thread::spawn(move || {
        let mut stream = write_stream;
        // Drain subscriber channel and forward to socket.
        loop {
            match subscriber_rx.recv() {
                Ok(data) => {
                    if stream.write_all(&data).is_err() {
                        break;
                    }
                    let _ = stream.flush();
                }
                Err(mpsc::RecvError) => {
                    // Reader thread dropped the sender or session exited.
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
                    // Write to PTY via shared writer.
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

        // Clean up: detach and update status.
        // This runs when the client disconnects (Ctrl-b d closes the socket).
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

    // Wait for either forwarding thread to exit.
    // This is the critical fix: we do NOT join socket_to_pty first.
    // If the PTY reader exits (child process done), pty_to_socket detects
    // subscriber channel disconnect and exits — we wake immediately.
    let _ = exit_rx.recv();

    // One side exited — shut down the socket to unblock the other thread.
    let _ = shutdown_stream.shutdown(std::net::Shutdown::Both);

    // Best-effort joins: both threads should exit promptly once the socket
    // is shut down.
    let _ = pty_to_socket_thread.join();
    let _ = socket_to_pty_thread.join();

    // Final cleanup: ensure session is not left in "attached" state.
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

fn handle_request(request: Request, registry: &SharedRegistry) -> Result<Response> {
    Ok(match request {
        Request::Ping => Response::ok(serde_json::json!("pong")),

        Request::ListSessions => {
            // Reap before listing so statuses are fresh.
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
                    Ok(_) => Response::ok(serde_json::json!({ "status": "created" })),
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

            // Auto-create the session entry if it doesn't exist yet.
            if !reg.exists(&name) {
                let session = Session::new(&name, &profile, &command, args.clone(), cwd.clone())?;
                reg.add(session)?;
            }

            match reg.spawn(&name, &command, args, cwd) {
                Ok(pid) => Response::ok(serde_json::json!({
                    "status": "spawned",
                    "pid": pid,
                })),
                Err(e) => {
                    reg.update(&name, Some(SessionStatus::Failed), None, None);
                    Response::error(&e.to_string())
                }
            }
        }

        Request::StopSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
            match reg.signal(&name, false) {
                Ok(_) => Response::ok(serde_json::json!({ "status": "stopped" })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::KillSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
            match reg.signal(&name, true) {
                Ok(_) => Response::ok(serde_json::json!({ "status": "killed" })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::ResizeSession { name, rows, cols } => {
            let reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
            match reg.resize(&name, rows, cols) {
                Ok(_) => Response::ok(serde_json::json!({ "status": "resized" })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::SendInput { name, data } => {
            let reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;
            match reg.write_input(&name, &data) {
                Ok(_) => Response::ok(serde_json::json!({ "status": "sent" })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::RestartSession { name } => {
            let mut reg = registry
                .lock()
                .map_err(|_| anyhow::anyhow!("Registry lock poisoned"))?;

            // Get session metadata before stopping.
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
                std::thread::sleep(std::time::Duration::from_millis(100));
            }

            // Force kill if still alive.
            if reg.is_alive(&name) {
                let _ = reg.signal(&name, true);
                std::thread::sleep(std::time::Duration::from_millis(200));
                reg.reap();
            }

            // Remove old session entry.
            reg.remove(&name);

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
                Ok(pid) => Response::ok(serde_json::json!({ "status": "restarted", "pid": pid })),
                Err(e) => Response::error(&e.to_string()),
            }
        }

        Request::AttachSession { .. } | Request::DetachSession { .. } => {
            // These are handled in handle_connection before reaching here.
            Response::error("Attach/Detach should not reach handle_request")
        }

        Request::Shutdown => {
            info!("Shutdown requested");
            state::remove_pid_file();
            let sock_path = state::socket_path()?;
            let _ = std::fs::remove_file(&sock_path);
            std::process::exit(0);
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
/// The caller should switch to raw byte I/O on the stream after this.
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

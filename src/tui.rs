use std::io;

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;

use crate::daemon::protocol::Request;
use crate::daemon::server;
use crate::daemon::session::SessionStatus;

/// Session info for TUI display.
#[derive(Debug, Clone)]
struct SessionInfo {
    name: String,
    profile: String,
    #[allow(dead_code)]
    command: String,
    cwd: Option<String>,
    #[allow(dead_code)]
    pid: Option<u64>,
    status: String,
}

/// Fetch sessions from daemon.
fn fetch_sessions() -> Vec<SessionInfo> {
    let resp = match server::send_request(&Request::ListSessions) {
        Ok(r) => r,
        Err(e) => {
            return vec![SessionInfo {
                name: format!("Error: {}", e),
                profile: String::new(),
                command: String::new(),
                cwd: None,
                pid: None,
                status: String::new(),
            }];
        }
    };

    let arr = resp.data.as_array();
    match arr {
        None => vec![],
        Some(sessions) => sessions
            .iter()
            .map(|s| SessionInfo {
                name: s
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                profile: s
                    .get("profile")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                command: s
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
                cwd: s.get("cwd").and_then(|v| v.as_str()).map(|s| s.to_string()),
                pid: s.get("pid").and_then(|v| v.as_u64()),
                status: s
                    .get("status")
                    .and_then(|v| v.as_str())
                    .unwrap_or("-")
                    .to_string(),
            })
            .collect(),
    }
}

/// Convert status string to a display color.
fn status_color(status: &str) -> Color {
    match status {
        "running" => Color::Green,
        "attached" => Color::Yellow,
        "detached" => Color::Cyan,
        "exited" => Color::DarkGray,
        "failed" => Color::Red,
        _ => Color::White,
    }
}

/// Run the TUI session switcher.
pub fn run_tui() -> Result<()> {
    // Auto-start daemon so TUI works from a fresh shell without a manual
    // `agentmux daemon` step.
    crate::daemon::autostart::ensure_daemon_running()?;

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_tui_loop(&mut terminal);

    // Restore terminal
    disable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, LeaveAlternateScreen)?;

    result
}

fn run_tui_loop<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>) -> Result<()> {
    let mut list_state = ListState::default();
    list_state.select(Some(0));

    // Confirm kill mode
    let mut confirm_kill: Option<usize> = None;
    let mut message: Option<String> = None;
    let mut message_tick: u64 = 0;

    loop {
        // Refresh sessions
        let sessions = fetch_sessions();
        let selected = list_state.selected().unwrap_or(0);

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(5), Constraint::Length(3)])
                .split(f.size());

            // Session list
            let items: Vec<ListItem> = sessions
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let status_str = format!("[{:>8}]", s.status);
                    let name_str = format!(" {:<15}", s.name);
                    let profile_str = format!("{:<12}", s.profile);
                    let cwd_str = s.cwd.as_deref().unwrap_or("-");

                    let color = status_color(&s.status);

                    let line = Line::from(vec![
                        Span::styled(
                            status_str,
                            Style::default().fg(color).add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(
                            name_str,
                            Style::default().fg(if i == selected {
                                Color::Yellow
                            } else {
                                Color::White
                            }),
                        ),
                        Span::styled(profile_str, Style::default().fg(Color::Cyan)),
                        Span::raw(cwd_str),
                    ]);

                    if let Some(kill_idx) = confirm_kill {
                        if i == kill_idx {
                            return ListItem::new(Line::from(vec![Span::styled(
                                format!(" ⚠ Kill '{}'? [y/N] ", s.name),
                                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                            )]));
                        }
                    }

                    ListItem::new(line)
                })
                .collect();

            let title = if confirm_kill.is_some() {
                "AgentMux Sessions — ⚠ KILL CONFIRMATION"
            } else {
                "AgentMux Sessions"
            };

            let list = List::new(items)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(title)
                        .border_style(if confirm_kill.is_some() {
                            Style::default().fg(Color::Red)
                        } else {
                            Style::default()
                        }),
                )
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                );

            f.render_stateful_widget(list, chunks[0], &mut list_state);

            // Help bar
            let help_text = if confirm_kill.is_some() {
                "y=confirm kill  n/Esc=cancel  q=quit"
            } else if let Some(msg) = &message {
                msg.as_str()
            } else {
                "Enter=attach  s=stop  k=kill  r=refresh  q=quit"
            };
            let help = Paragraph::new(help_text)
                .block(Block::default().borders(Borders::ALL).title("Controls"))
                .wrap(Wrap { trim: true });
            f.render_widget(help, chunks[1]);
        })?;

        // Clear message after a few ticks
        if message.is_some() {
            message_tick += 1;
            if message_tick > 50 {
                message = None;
                message_tick = 0;
            }
        }

        // Poll for events with timeout (for periodic refresh).
        if !event::poll(std::time::Duration::from_millis(200))? {
            continue;
        }

        if let Event::Key(key) = event::read()? {
            // Handle confirm kill mode
            if confirm_kill.is_some() {
                match key.code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let idx = confirm_kill.unwrap();
                        if idx < sessions.len() {
                            let name = &sessions[idx].name;
                            match send_simple_request(&Request::KillSession { name: name.clone() })
                            {
                                Ok(_) => {
                                    message = Some(format!("Killed: {}", name));
                                }
                                Err(e) => {
                                    message = Some(format!("Error: {}", e));
                                }
                            }
                        }
                        confirm_kill = None;
                        message_tick = 0;
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Enter => {
                        confirm_kill = None;
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        return Ok(());
                    }
                    _ => {}
                }
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    return Ok(());
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if !sessions.is_empty() {
                        let i = list_state.selected().unwrap_or(0);
                        list_state.select(Some((i + 1) % sessions.len()));
                    }
                }
                KeyCode::Up => {
                    if !sessions.is_empty() {
                        let i = list_state.selected().unwrap_or(0);
                        let len = sessions.len();
                        list_state.select(Some((i + len - 1) % len));
                    }
                }
                KeyCode::Enter => {
                    if selected < sessions.len() {
                        let name = sessions[selected].name.clone();
                        // Temporarily exit TUI to attach
                        disable_raw_mode()?;
                        let mut stdout = io::stdout();
                        execute!(stdout, LeaveAlternateScreen)?;

                        let attach_result = attach_from_tui(&name);

                        // Re-enter TUI
                        enable_raw_mode()?;
                        let mut stdout = io::stdout();
                        execute!(stdout, EnterAlternateScreen)?;
                        // Clear screen
                        terminal.clear()?;

                        match attach_result {
                            Ok(()) => {
                                message = Some(format!("Detached from: {}", name));
                            }
                            Err(e) => {
                                message = Some(format!("Attach error: {}", e));
                            }
                        }
                        message_tick = 0;
                    }
                }
                KeyCode::Char('s') => {
                    if selected < sessions.len() {
                        let name = sessions[selected].name.clone();
                        match send_simple_request(&Request::StopSession { name: name.clone() }) {
                            Ok(_) => {
                                message = Some(format!("Stopped: {}", name));
                            }
                            Err(e) => {
                                message = Some(format!("Error: {}", e));
                            }
                        }
                        message_tick = 0;
                    }
                }
                KeyCode::Char('k') => {
                    if selected < sessions.len() {
                        confirm_kill = Some(selected);
                    }
                }
                KeyCode::Char('r') => {
                    // Just let the loop refresh.
                }
                _ => {}
            }
        }
    }
}

/// Send a request and ignore the response data (just check ok).
fn send_simple_request(request: &Request) -> Result<()> {
    let resp = server::send_request(request)?;
    if resp.ok {
        Ok(())
    } else {
        anyhow::bail!(
            "{}",
            resp.data
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
        )
    }
}

/// Attach to a session from within TUI.
/// This suspends the TUI and enters raw mode attach.
fn attach_from_tui(session_name: &str) -> Result<()> {
    let stream = server::connect_attach(session_name)?;
    crate::pty::attach::attach_to_session(stream, session_name)
}

// Suppress unused import warnings
#[allow(dead_code)]
fn _suppress_unused() {
    let _: SessionStatus = SessionStatus::Running;
    let _ = Constraint::Percentage(100);
}

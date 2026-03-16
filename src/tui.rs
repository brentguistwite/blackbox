use std::time::{Duration, Instant};

use chrono::Utc;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    DefaultTerminal, Frame,
};

/// App state for the live TUI dashboard.
pub struct App {
    pub running: bool,
    pub daemon_running: bool,
    pub last_refresh: Instant,
    pub error: Option<String>,
}

impl Default for App {
    fn default() -> Self {
        Self {
            running: true,
            daemon_running: false,
            last_refresh: Instant::now(),
            error: None,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// Check if daemon is running via PID file.
    pub fn refresh_daemon_status(&mut self) {
        self.daemon_running = match crate::config::data_dir() {
            Ok(data_dir) => crate::daemon::is_daemon_running(&data_dir)
                .ok()
                .flatten()
                .is_some(),
            Err(_) => false,
        };
        self.last_refresh = Instant::now();
    }

    fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        match (code, modifiers) {
            (KeyCode::Char('q'), _) => self.running = false,
            (KeyCode::Char('c'), KeyModifiers::CONTROL) => self.running = false,
            _ => {}
        }
    }
}

/// Entry point: initialize terminal, run event loop, restore on exit.
pub fn run_live() -> anyhow::Result<()> {
    let mut app = App::new();

    // Validate config + DB access up front; store error in app state if unavailable
    if let Err(e) = crate::config::load_config() {
        app.error = Some(format!("Config error: {e}"));
    } else if let Err(e) = crate::config::data_dir().and_then(|d| {
        let db_path = d.join("blackbox.db");
        crate::db::open_db(&db_path).map_err(|e| anyhow::anyhow!(e))
    }) {
        app.error = Some(format!("DB error: {e}"));
    }

    app.refresh_daemon_status();

    let mut terminal = ratatui::init();
    let result = run_event_loop(&mut terminal, &mut app);
    ratatui::restore();
    result
}

/// Main event loop: poll input → tick refresh → render.
fn run_event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> anyhow::Result<()> {
    let tick_rate = Duration::from_secs(1);

    while app.running {
        terminal.draw(|frame| render(frame, app))?;

        let timeout = tick_rate.saturating_sub(app.last_refresh.elapsed());
        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == event::KeyEventKind::Press
        {
            app.handle_key(key.code, key.modifiers);
        }

        // Tick: refresh data every second
        if app.last_refresh.elapsed() >= tick_rate {
            app.refresh_daemon_status();
        }
    }
    Ok(())
}

/// Render the three-row layout: header, body, footer.
fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let [header_area, body_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    render_header(frame, header_area, app);
    render_body(frame, body_area, app);
    render_footer(frame, footer_area);
}

fn render_header(frame: &mut Frame, area: Rect, app: &App) {
    let now = Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let daemon_status = if app.daemon_running {
        Span::styled(" daemon: running ", Style::default().fg(Color::Green))
    } else {
        Span::styled(" daemon: stopped ", Style::default().fg(Color::Red))
    };
    let line = Line::from(vec![
        Span::styled(
            " blackbox live ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {now} ")),
        daemon_status,
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn render_body(frame: &mut Frame, area: Rect, app: &App) {
    let content = if let Some(ref err) = app.error {
        vec![
            Line::raw(""),
            Line::styled(
                format!("  Error: {err}"),
                Style::default().fg(Color::Red),
            ),
            Line::raw(""),
            Line::raw("  Run 'blackbox setup' to configure, or check your data directory."),
        ]
    } else {
        vec![
            Line::raw(""),
            Line::styled(
                "  Dashboard data panels coming in next update.",
                Style::default().fg(Color::DarkGray),
            ),
        ]
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Activity ");
    frame.render_widget(Paragraph::new(content).block(block), area);
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let line = Line::from(vec![
        Span::styled(" q", Style::default().fg(Color::Yellow)),
        Span::raw(" quit  "),
        Span::styled("r", Style::default().fg(Color::Yellow)),
        Span::raw(" refresh  "),
        Span::styled("j/k", Style::default().fg(Color::Yellow)),
        Span::raw(" scroll"),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_new_defaults() {
        let app = App::new();
        assert!(app.running);
        assert!(!app.daemon_running);
        assert!(app.error.is_none());
    }

    #[test]
    fn test_quit_on_q() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('q'), KeyModifiers::NONE);
        assert!(!app.running);
    }

    #[test]
    fn test_quit_on_ctrl_c() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert!(!app.running);
    }

    #[test]
    fn test_other_keys_dont_quit() {
        let mut app = App::new();
        app.handle_key(KeyCode::Char('j'), KeyModifiers::NONE);
        assert!(app.running);
        app.handle_key(KeyCode::Char('k'), KeyModifiers::NONE);
        assert!(app.running);
        app.handle_key(KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.running);
    }

    #[test]
    fn test_app_with_error() {
        let mut app = App::new();
        app.error = Some("test error".into());
        assert!(app.error.is_some());
        assert!(app.running);
    }

    #[test]
    fn test_render_does_not_panic() {
        // Verify rendering with a backend that doesn't require a real terminal
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let app = App::new();
        terminal.draw(|frame| render(frame, &app)).unwrap();
    }

    #[test]
    fn test_render_with_error_does_not_panic() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.error = Some("Config error: file not found".into());
        terminal.draw(|frame| render(frame, &app)).unwrap();
    }

    #[test]
    fn test_render_daemon_running() {
        let backend = ratatui::backend::TestBackend::new(80, 24);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();

        let mut app = App::new();
        app.daemon_running = true;
        terminal.draw(|frame| render(frame, &app)).unwrap();
    }
}

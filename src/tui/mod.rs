use crate::app::App;
use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;

pub struct Tui {
    pub terminal: Terminal<CrosstermBackend<io::Stdout>>,
}

impl Tui {
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal })
    }

    pub fn draw(&mut self, app: &App) -> Result<()> {
        self.terminal.draw(|f| ui(f, app))?;
        Ok(())
    }

    pub fn restore(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let _ = self.restore();
    }
}

fn ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(0),
            Constraint::Length(3),
        ])
        .split(f.area());

    render_header(f, chunks[0], app);
    render_file_list(f, chunks[1], app);
    render_footer(f, chunks[2], app);
}

fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(&app.connection_string, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Yellow)),
            Span::raw(&app.current_path),
        ]),
        Line::from(vec![
            Span::styled("Actions: ", Style::default().fg(Color::Green)),
            Span::raw("Enter=Open  d=Download  Del=Delete  q=Quit"),
        ]),
    ])
    .block(Block::default().borders(Borders::ALL).title("bssh"));

    f.render_widget(header, area);
}

fn render_file_list(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app
        .files
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let icon = if file.is_dir { "📁" } else { "📄" };
            let size = if file.is_dir {
                String::from("<DIR>")
            } else {
                format_size(file.size)
            };

            let content = Line::from(vec![
                Span::raw(format!("{} ", icon)),
                Span::styled(
                    format!("{:<40}", file.name),
                    if file.is_dir {
                        Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    format!("{:>10}", size),
                    Style::default().fg(Color::DarkGray),
                ),
            ]);

            let style = if i == app.selected_index {
                Style::default().bg(Color::DarkGray).fg(Color::White)
            } else {
                Style::default()
            };

            ListItem::new(content).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Files"));

    f.render_widget(list, area);
}

fn render_footer(f: &mut Frame, area: Rect, app: &App) {
    let help_text = if app.status_message.is_empty() {
        vec![
            Line::from(vec![
                Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
                Span::raw(": Navigate  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(": Open  "),
                Span::styled("d", Style::default().fg(Color::Yellow)),
                Span::raw(": Download  "),
                Span::styled("u", Style::default().fg(Color::Yellow)),
                Span::raw(": Upload  "),
                Span::styled("n", Style::default().fg(Color::Yellow)),
                Span::raw(": New Dir  "),
                Span::styled("r", Style::default().fg(Color::Yellow)),
                Span::raw(": Rename  "),
            ]),
            Line::from(vec![
                Span::styled("Del", Style::default().fg(Color::Yellow)),
                Span::raw(": Delete  "),
                Span::styled("e", Style::default().fg(Color::Yellow)),
                Span::raw(": Execute  "),
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::raw(": Quit"),
            ]),
        ]
    } else {
        vec![Line::from(Span::styled(
            &app.status_message,
            Style::default().fg(Color::Green),
        ))]
    };

    let footer = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .alignment(Alignment::Left);

    f.render_widget(footer, area);
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit_index = 0;

    while size >= 1024.0 && unit_index < UNITS.len() - 1 {
        size /= 1024.0;
        unit_index += 1;
    }

    if unit_index == 0 {
        format!("{} {}", size as u64, UNITS[unit_index])
    } else {
        format!("{:.1} {}", size, UNITS[unit_index])
    }
}

pub enum InputAction {
    MoveUp,
    MoveDown,
    Enter,
    Download,
    Upload,
    NewDirectory,
    Rename,
    Delete,
    Execute,
    Quit,
    None,
}

pub fn handle_input() -> Result<InputAction> {
    if event::poll(std::time::Duration::from_millis(100))? {
        if let Event::Key(key) = event::read()? {
            return Ok(match key.code {
                KeyCode::Up | KeyCode::Char('k') => InputAction::MoveUp,
                KeyCode::Down | KeyCode::Char('j') => InputAction::MoveDown,
                KeyCode::Enter => InputAction::Enter,
                KeyCode::Char('d') => InputAction::Download,
                KeyCode::Char('u') => InputAction::Upload,
                KeyCode::Char('n') => InputAction::NewDirectory,
                KeyCode::Char('r') => InputAction::Rename,
                KeyCode::Delete | KeyCode::Char('x') => InputAction::Delete,
                KeyCode::Char('e') => InputAction::Execute,
                KeyCode::Char('q') | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    InputAction::Quit
                }
                _ => InputAction::None,
            });
        }
    }
    Ok(InputAction::None)
}

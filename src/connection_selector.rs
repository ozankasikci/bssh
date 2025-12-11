use crate::connections::SavedConnection;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame, Terminal,
};
use std::io;

pub struct ConnectionSelector {
    connections: Vec<SavedConnection>,
    selected_index: usize,
}

impl ConnectionSelector {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected_index: 0,
        }
    }

    pub fn run(mut self) -> Result<Option<SavedConnection>> {
        if self.connections.is_empty() {
            println!("No saved connections found.");
            println!("\nUsage: bssh [OPTIONS] <DESTINATION> [PATH]");
            println!("\nExample: bssh user@hostname");
            return Ok(None);
        }

        let mut terminal = setup_terminal()?;
        let result = self.run_selector(&mut terminal)?;
        restore_terminal(&mut terminal)?;

        Ok(result)
    }

    fn run_selector(
        &mut self,
        terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ) -> Result<Option<SavedConnection>> {
        loop {
            terminal.draw(|f| self.render(f))?;

            if let Event::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q')
                    | KeyCode::Esc
                    | KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        return Ok(None);
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        if self.selected_index > 0 {
                            self.selected_index -= 1;
                        }
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if self.selected_index < self.connections.len() - 1 {
                            self.selected_index += 1;
                        }
                    }
                    KeyCode::Enter => {
                        return Ok(Some(self.connections[self.selected_index].clone()));
                    }
                    _ => {}
                }
            }
        }
    }

    fn render(&self, f: &mut Frame) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(0),
                Constraint::Length(3),
            ])
            .split(f.area());

        // Header
        let header = Paragraph::new(vec![
            Line::from(vec![
                Span::styled(
                    "Select SSH Connection",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![Span::raw(format!(
                "{} saved connection(s)",
                self.connections.len()
            ))]),
        ])
        .block(Block::default().borders(Borders::ALL).title("bssh"));

        f.render_widget(header, chunks[0]);

        // Connection list
        let items: Vec<ListItem> = self
            .connections
            .iter()
            .enumerate()
            .map(|(i, conn)| {
                let line = Line::from(vec![
                    Span::styled(
                        format!("{:<20}", conn.name),
                        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::raw(conn.display_name()),
                ]);

                let style = if i == self.selected_index {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                } else {
                    Style::default()
                };

                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Connections"));

        f.render_widget(list, chunks[1]);

        // Footer
        let footer = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
                Span::raw(": Navigate  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(": Connect  "),
                Span::styled("q", Style::default().fg(Color::Yellow)),
                Span::raw(": Quit"),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .alignment(Alignment::Left);

        f.render_widget(footer, chunks[2]);
    }
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
    crossterm::terminal::enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        terminal.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

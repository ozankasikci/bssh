use crate::connections::SavedConnection;
use anyhow::Result;
use arboard::Clipboard;
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
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum SelectorResult {
    Connect(SavedConnection),
    Edit(SavedConnection),
    Cancel,
}

pub struct ConnectionSelector {
    connections: Vec<SavedConnection>,
    selected_index: usize,
    status_message: Option<(String, Instant)>,
    edit_form: Option<EditForm>,
}

impl ConnectionSelector {
    pub fn new(connections: Vec<SavedConnection>) -> Self {
        Self {
            connections,
            selected_index: 0,
            status_message: None,
            edit_form: None,
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
                // Handle edit mode input
                if self.edit_form.is_some() {
                    match key.code {
                        KeyCode::Esc => {
                            self.edit_form = None;
                            self.status_message = Some(("Edit cancelled".to_string(), Instant::now()));
                        }
                        KeyCode::Enter => {
                            if let Some(ref form) = self.edit_form {
                                match form.to_connection() {
                                    Ok(updated) => {
                                        let original_name = form.original_name.clone();
                                        // Update the connection in the list
                                        if let Some(conn) = self.connections.iter_mut().find(|c| c.name == original_name) {
                                            *conn = updated.clone();
                                        }
                                        // Save to file
                                        if let Err(e) = crate::connections::update_connection(&original_name, updated) {
                                            self.status_message = Some((format!("Save failed: {}", e), Instant::now()));
                                        } else {
                                            self.status_message = Some(("Connection saved".to_string(), Instant::now()));
                                        }
                                        self.edit_form = None;
                                    }
                                    Err(e) => {
                                        self.status_message = Some((format!("Invalid: {}", e), Instant::now()));
                                    }
                                }
                            }
                        }
                        KeyCode::Tab | KeyCode::Down => {
                            if let Some(ref mut form) = self.edit_form {
                                form.next_field();
                            }
                        }
                        KeyCode::BackTab | KeyCode::Up => {
                            if let Some(ref mut form) = self.edit_form {
                                form.prev_field();
                            }
                        }
                        KeyCode::Backspace => {
                            if let Some(ref mut form) = self.edit_form {
                                form.delete_char();
                            }
                        }
                        KeyCode::Char(c) => {
                            if let Some(ref mut form) = self.edit_form {
                                form.insert_char(c);
                            }
                        }
                        _ => {}
                    }
                    continue;
                }

                // Normal mode input
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => {
                        return Ok(None);
                    }
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
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
                    KeyCode::Char('c') => {
                        let conn = &self.connections[self.selected_index];
                        let ssh_cmd = conn.ssh_command();
                        match Clipboard::new().and_then(|mut cb| cb.set_text(&ssh_cmd)) {
                            Ok(_) => {
                                self.status_message = Some((
                                    format!("Copied: {}", ssh_cmd),
                                    Instant::now(),
                                ));
                            }
                            Err(_) => {
                                self.status_message = Some((
                                    "Failed to copy to clipboard".to_string(),
                                    Instant::now(),
                                ));
                            }
                        }
                    }
                    KeyCode::Char('e') => {
                        let conn = &self.connections[self.selected_index];
                        self.edit_form = Some(EditForm::from_connection(conn));
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
        // If in edit mode, render the edit form
        if let Some(ref form) = self.edit_form {
            self.render_edit_form(f, form);
            return;
        }

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

        // Footer - show status message if recent, otherwise show help
        let footer_content = if let Some((ref msg, timestamp)) = self.status_message {
            if timestamp.elapsed() < Duration::from_secs(2) {
                Line::from(vec![
                    Span::styled(msg.clone(), Style::default().fg(Color::Green)),
                ])
            } else {
                Self::help_line()
            }
        } else {
            Self::help_line()
        };

        let footer = Paragraph::new(vec![footer_content])
            .block(Block::default().borders(Borders::ALL).title("Help"))
            .alignment(Alignment::Left);

        f.render_widget(footer, chunks[2]);
    }

    fn render_edit_form(&self, f: &mut Frame, form: &EditForm) {
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
                    "Edit Connection",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                ),
            ]),
            Line::from(vec![Span::raw(format!("Editing: {}", form.original_name))]),
        ])
        .block(Block::default().borders(Borders::ALL).title("bssh"));

        f.render_widget(header, chunks[0]);

        // Edit form fields
        let fields = [
            ("Name", &form.name, EditField::Name),
            ("Host", &form.host, EditField::Host),
            ("Port", &form.port, EditField::Port),
            ("Username", &form.username, EditField::Username),
            ("Identity File", &form.identity_file, EditField::IdentityFile),
        ];

        let items: Vec<ListItem> = fields
            .iter()
            .map(|(label, value, field)| {
                let is_selected = form.current_field == *field;
                let cursor = if is_selected { "█" } else { "" };

                let line = Line::from(vec![
                    Span::styled(
                        format!("{:<14}", label),
                        Style::default().fg(Color::Yellow),
                    ),
                    Span::raw(": "),
                    Span::raw(format!("{}{}", value, cursor)),
                ]);

                let style = if is_selected {
                    Style::default().bg(Color::DarkGray).fg(Color::White)
                } else {
                    Style::default()
                };

                ListItem::new(line).style(style)
            })
            .collect();

        let list = List::new(items).block(Block::default().borders(Borders::ALL).title("Fields"));

        f.render_widget(list, chunks[1]);

        // Footer with edit mode help
        let footer = Paragraph::new(vec![
            Line::from(vec![
                Span::styled("Tab/↑↓", Style::default().fg(Color::Yellow)),
                Span::raw(": Navigate  "),
                Span::styled("Enter", Style::default().fg(Color::Yellow)),
                Span::raw(": Save  "),
                Span::styled("Esc", Style::default().fg(Color::Yellow)),
                Span::raw(": Cancel"),
            ]),
        ])
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .alignment(Alignment::Left);

        f.render_widget(footer, chunks[2]);
    }

    fn help_line() -> Line<'static> {
        Line::from(vec![
            Span::styled("↑/↓", Style::default().fg(Color::Yellow)),
            Span::raw(": Navigate  "),
            Span::styled("e", Style::default().fg(Color::Yellow)),
            Span::raw(": Edit  "),
            Span::styled("c", Style::default().fg(Color::Yellow)),
            Span::raw(": Copy  "),
            Span::styled("Enter", Style::default().fg(Color::Yellow)),
            Span::raw(": Connect  "),
            Span::styled("q", Style::default().fg(Color::Yellow)),
            Span::raw(": Quit"),
        ])
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

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditField {
    Name,
    Host,
    Port,
    Username,
    IdentityFile,
}

impl EditField {
    fn next(self) -> Self {
        match self {
            EditField::Name => EditField::Host,
            EditField::Host => EditField::Port,
            EditField::Port => EditField::Username,
            EditField::Username => EditField::IdentityFile,
            EditField::IdentityFile => EditField::Name,
        }
    }

    fn prev(self) -> Self {
        match self {
            EditField::Name => EditField::IdentityFile,
            EditField::Host => EditField::Name,
            EditField::Port => EditField::Host,
            EditField::Username => EditField::Port,
            EditField::IdentityFile => EditField::Username,
        }
    }
}

pub struct EditForm {
    pub name: String,
    pub host: String,
    pub port: String,
    pub username: String,
    pub identity_file: String,
    pub current_field: EditField,
    pub original_name: String,
}

impl EditForm {
    pub fn from_connection(conn: &SavedConnection) -> Self {
        Self {
            name: conn.name.clone(),
            host: conn.host.clone(),
            port: conn.port.to_string(),
            username: conn.username.clone(),
            identity_file: conn.identity_file.as_ref().map(|p| p.display().to_string()).unwrap_or_default(),
            current_field: EditField::Name,
            original_name: conn.name.clone(),
        }
    }

    pub fn current_value(&self) -> &str {
        match self.current_field {
            EditField::Name => &self.name,
            EditField::Host => &self.host,
            EditField::Port => &self.port,
            EditField::Username => &self.username,
            EditField::IdentityFile => &self.identity_file,
        }
    }

    pub fn current_value_mut(&mut self) -> &mut String {
        match self.current_field {
            EditField::Name => &mut self.name,
            EditField::Host => &mut self.host,
            EditField::Port => &mut self.port,
            EditField::Username => &mut self.username,
            EditField::IdentityFile => &mut self.identity_file,
        }
    }

    pub fn next_field(&mut self) {
        self.current_field = self.current_field.next();
    }

    pub fn prev_field(&mut self) {
        self.current_field = self.current_field.prev();
    }

    pub fn insert_char(&mut self, c: char) {
        self.current_value_mut().push(c);
    }

    pub fn delete_char(&mut self) {
        self.current_value_mut().pop();
    }

    pub fn to_connection(&self) -> Result<SavedConnection> {
        let port: u16 = self.port.parse().map_err(|_| anyhow::anyhow!("Invalid port number"))?;
        let identity_file = if self.identity_file.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(&self.identity_file))
        };
        Ok(SavedConnection::new(
            self.name.clone(),
            self.host.clone(),
            port,
            self.username.clone(),
            identity_file,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_edit_form_from_connection() {
        let conn = SavedConnection::new(
            "myserver".to_string(),
            "example.com".to_string(),
            22,
            "admin".to_string(),
            Some(PathBuf::from("/home/user/.ssh/id_rsa")),
        );

        let form = EditForm::from_connection(&conn);

        assert_eq!(form.name, "myserver");
        assert_eq!(form.host, "example.com");
        assert_eq!(form.port, "22");
        assert_eq!(form.username, "admin");
        assert_eq!(form.identity_file, "/home/user/.ssh/id_rsa");
        assert_eq!(form.current_field, EditField::Name);
    }

    #[test]
    fn test_edit_form_navigate_fields() {
        let conn = SavedConnection::new("s".to_string(), "h".to_string(), 22, "u".to_string(), None);
        let mut form = EditForm::from_connection(&conn);

        assert_eq!(form.current_field, EditField::Name);
        form.next_field();
        assert_eq!(form.current_field, EditField::Host);
        form.next_field();
        assert_eq!(form.current_field, EditField::Port);
        form.next_field();
        assert_eq!(form.current_field, EditField::Username);
        form.next_field();
        assert_eq!(form.current_field, EditField::IdentityFile);
        form.next_field();
        assert_eq!(form.current_field, EditField::Name); // wraps around

        form.prev_field();
        assert_eq!(form.current_field, EditField::IdentityFile);
    }

    #[test]
    fn test_edit_form_insert_delete_chars() {
        let conn = SavedConnection::new("server".to_string(), "host".to_string(), 22, "user".to_string(), None);
        let mut form = EditForm::from_connection(&conn);

        // Edit name field
        form.insert_char('1');
        assert_eq!(form.name, "server1");

        form.delete_char();
        assert_eq!(form.name, "server");

        // Switch to host and edit
        form.next_field();
        form.insert_char('.');
        form.insert_char('c');
        form.insert_char('o');
        form.insert_char('m');
        assert_eq!(form.host, "host.com");
    }

    #[test]
    fn test_edit_form_to_connection() {
        let conn = SavedConnection::new("s".to_string(), "h".to_string(), 22, "u".to_string(), None);
        let mut form = EditForm::from_connection(&conn);

        // Modify values
        form.name = "newserver".to_string();
        form.host = "newhost.com".to_string();
        form.port = "2222".to_string();
        form.username = "newuser".to_string();
        form.identity_file = "/path/to/key".to_string();

        let updated = form.to_connection().unwrap();

        assert_eq!(updated.name, "newserver");
        assert_eq!(updated.host, "newhost.com");
        assert_eq!(updated.port, 2222);
        assert_eq!(updated.username, "newuser");
        assert_eq!(updated.identity_file, Some(PathBuf::from("/path/to/key")));
    }

    #[test]
    fn test_edit_form_invalid_port_returns_error() {
        let conn = SavedConnection::new("s".to_string(), "h".to_string(), 22, "u".to_string(), None);
        let mut form = EditForm::from_connection(&conn);

        form.port = "invalid".to_string();

        let result = form.to_connection();
        assert!(result.is_err());
    }
}

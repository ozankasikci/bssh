use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use russh_sftp::client::SftpSession;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Debug, Clone, PartialEq)]
pub enum EditorMode {
    Normal,
    Insert,
    Command,
    Search,
}

pub struct EditorState {
    pub buffer: Vec<String>,
    pub cursor_row: usize,
    pub cursor_col: usize,
    pub mode: EditorMode,
    pub yank_register: Vec<String>,
    pub status_message: String,
    pub command_buffer: String,
    pub search_pattern: String,
    pub scroll_offset: usize,
    pub filename: String,
    pub remote_path: String,
    pub modified: bool,
    pub should_quit: bool,
}

impl EditorState {
    pub fn new(filename: String, remote_path: String, content: String) -> Self {
        let buffer: Vec<String> = if content.is_empty() {
            vec![String::new()]
        } else {
            content.lines().map(|s| s.to_string()).collect()
        };

        Self {
            buffer,
            cursor_row: 0,
            cursor_col: 0,
            mode: EditorMode::Normal,
            yank_register: Vec::new(),
            status_message: String::from("Normal mode"),
            command_buffer: String::new(),
            search_pattern: String::new(),
            scroll_offset: 0,
            filename,
            remote_path,
            modified: false,
            should_quit: false,
        }
    }

    pub fn get_current_line(&self) -> &str {
        self.buffer.get(self.cursor_row).map(|s| s.as_str()).unwrap_or("")
    }

    pub fn get_current_line_mut(&mut self) -> &mut String {
        &mut self.buffer[self.cursor_row]
    }

    fn clamp_cursor(&mut self) {
        if self.cursor_row >= self.buffer.len() {
            self.cursor_row = self.buffer.len().saturating_sub(1);
        }

        let line_len = self.buffer.get(self.cursor_row).map(|s| s.len()).unwrap_or(0);
        let max_col = if self.mode == EditorMode::Insert {
            line_len
        } else {
            line_len.saturating_sub(1).max(0)
        };

        if self.cursor_col > max_col {
            self.cursor_col = max_col;
        }
    }

    pub fn move_cursor_up(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.clamp_cursor();
        }
    }

    pub fn move_cursor_down(&mut self) {
        if self.cursor_row < self.buffer.len() - 1 {
            self.cursor_row += 1;
            self.clamp_cursor();
        }
    }

    pub fn move_cursor_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.get_current_line().len();
            if self.mode == EditorMode::Normal && self.cursor_col > 0 {
                self.cursor_col -= 1;
            }
        }
    }

    pub fn move_cursor_right(&mut self) {
        let line_len = self.get_current_line().len();
        let max_col = if self.mode == EditorMode::Insert {
            line_len
        } else {
            line_len.saturating_sub(1)
        };

        if self.cursor_col < max_col {
            self.cursor_col += 1;
        } else if self.cursor_row < self.buffer.len() - 1 {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    pub fn move_to_line_start(&mut self) {
        self.cursor_col = 0;
    }

    pub fn move_to_line_end(&mut self) {
        let line_len = self.get_current_line().len();
        self.cursor_col = if self.mode == EditorMode::Insert {
            line_len
        } else {
            line_len.saturating_sub(1).max(0)
        };
    }

    pub fn move_to_buffer_start(&mut self) {
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    pub fn move_to_buffer_end(&mut self) {
        self.cursor_row = self.buffer.len().saturating_sub(1);
        self.move_to_line_end();
    }

    pub fn delete_line(&mut self) {
        if self.buffer.len() == 1 {
            self.yank_register = vec![self.buffer[0].clone()];
            self.buffer[0].clear();
        } else {
            self.yank_register = vec![self.buffer.remove(self.cursor_row)];
            if self.cursor_row >= self.buffer.len() {
                self.cursor_row = self.buffer.len() - 1;
            }
        }
        self.clamp_cursor();
        self.modified = true;
        self.status_message = String::from("Line deleted");
    }

    pub fn yank_line(&mut self) {
        self.yank_register = vec![self.buffer[self.cursor_row].clone()];
        self.status_message = String::from("Line yanked");
    }

    pub fn paste_below(&mut self) {
        if !self.yank_register.is_empty() {
            for (i, line) in self.yank_register.iter().enumerate() {
                self.buffer.insert(self.cursor_row + 1 + i, line.clone());
            }
            self.cursor_row += 1;
            self.modified = true;
            self.status_message = String::from("Pasted");
        }
    }

    pub fn insert_char(&mut self, c: char) {
        let cursor_col = self.cursor_col;
        let line = self.get_current_line_mut();
        if cursor_col >= line.len() {
            line.push(c);
        } else {
            line.insert(cursor_col, c);
        }
        self.cursor_col += 1;
        self.modified = true;
    }

    pub fn delete_char(&mut self) {
        let cursor_col = self.cursor_col;
        if cursor_col > 0 {
            let line = self.get_current_line_mut();
            line.remove(cursor_col - 1);
            self.cursor_col -= 1;
            self.modified = true;
        } else if self.cursor_row > 0 {
            let current_line = self.buffer.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.buffer[self.cursor_row].len();
            self.buffer[self.cursor_row].push_str(&current_line);
            self.modified = true;
        }
    }

    pub fn insert_newline(&mut self) {
        let cursor_col = self.cursor_col;
        let line = self.get_current_line_mut();
        let remainder = line.split_off(cursor_col);
        self.buffer.insert(self.cursor_row + 1, remainder);
        self.cursor_row += 1;
        self.cursor_col = 0;
        self.modified = true;
    }

    pub fn execute_command(&mut self, command: &str) {
        match command {
            "w" | "write" => {
                self.status_message = String::from("Saving...");
            }
            "q" | "quit" => {
                if self.modified {
                    self.status_message = String::from("No write since last change (use :q! to override)");
                } else {
                    self.should_quit = true;
                }
            }
            "q!" => {
                self.should_quit = true;
            }
            "wq" | "x" => {
                self.status_message = String::from("Saving and quitting...");
            }
            _ => {
                self.status_message = format!("Unknown command: {}", command);
            }
        }
    }

    pub fn update_scroll(&mut self, viewport_height: usize) {
        let margin = 3;

        if self.cursor_row < self.scroll_offset + margin {
            self.scroll_offset = self.cursor_row.saturating_sub(margin);
        }

        if self.cursor_row >= self.scroll_offset + viewport_height - margin {
            self.scroll_offset = self.cursor_row + margin - viewport_height + 1;
        }
    }
}

pub async fn load_file_content(sftp: &SftpSession, remote_path: &str) -> Result<String> {
    let mut file = sftp.open(remote_path).await?;
    let mut content = String::new();
    file.read_to_string(&mut content).await?;
    Ok(content)
}

pub async fn save_file_content(sftp: &SftpSession, remote_path: &str, content: &str) -> Result<()> {
    let mut file = sftp.create(remote_path).await?;
    file.write_all(content.as_bytes()).await?;
    Ok(())
}

pub fn render_editor(f: &mut Frame, area: Rect, editor: &EditorState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    // Header
    let mode_indicator = match editor.mode {
        EditorMode::Normal => Span::styled("NORMAL", Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
        EditorMode::Insert => Span::styled("INSERT", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        EditorMode::Command => Span::styled("COMMAND", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
        EditorMode::Search => Span::styled("SEARCH", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
    };

    let modified_indicator = if editor.modified { " [+]" } else { "" };
    let header = Line::from(vec![
        mode_indicator,
        Span::raw(" | "),
        Span::raw(&editor.filename),
        Span::raw(modified_indicator),
    ]);
    let header_widget = Paragraph::new(header);
    f.render_widget(header_widget, chunks[0]);

    // Editor area
    let viewport_height = chunks[1].height as usize;
    let visible_start = editor.scroll_offset;
    let visible_end = (visible_start + viewport_height).min(editor.buffer.len());

    let visible_lines: Vec<Line> = editor.buffer[visible_start..visible_end]
        .iter()
        .map(|line| Line::from(line.as_str()))
        .collect();

    let editor_widget = Paragraph::new(visible_lines)
        .block(Block::default().borders(Borders::NONE));
    f.render_widget(editor_widget, chunks[1]);

    // Footer
    let footer_text = match editor.mode {
        EditorMode::Command => format!(":{}", editor.command_buffer),
        EditorMode::Search => format!("/{}", editor.command_buffer),
        _ => editor.status_message.clone(),
    };
    let footer = Paragraph::new(footer_text);
    f.render_widget(footer, chunks[2]);

    // Set cursor position
    let cursor_screen_row = editor.cursor_row.saturating_sub(editor.scroll_offset);
    let cursor_x = chunks[1].x + editor.cursor_col as u16;
    let cursor_y = chunks[1].y + cursor_screen_row as u16;
    f.set_cursor_position((cursor_x, cursor_y));
}

pub fn handle_editor_input(editor: &mut EditorState) -> Result<bool> {
    if !event::poll(Duration::from_millis(100))? {
        return Ok(false);
    }

    if let Event::Key(key) = event::read()? {
        match editor.mode {
            EditorMode::Normal => handle_normal_mode(editor, key),
            EditorMode::Insert => handle_insert_mode(editor, key),
            EditorMode::Command | EditorMode::Search => handle_command_mode(editor, key),
        }
        return Ok(true);
    }

    Ok(false)
}

fn handle_normal_mode(editor: &mut EditorState, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            editor.should_quit = true;
        }
        KeyCode::Char('h') | KeyCode::Left => editor.move_cursor_left(),
        KeyCode::Char('j') | KeyCode::Down => editor.move_cursor_down(),
        KeyCode::Char('k') | KeyCode::Up => editor.move_cursor_up(),
        KeyCode::Char('l') | KeyCode::Right => editor.move_cursor_right(),
        KeyCode::Char('0') => editor.move_to_line_start(),
        KeyCode::Char('$') => editor.move_to_line_end(),
        KeyCode::Char('g') => {
            editor.move_to_buffer_start();
        }
        KeyCode::Char('G') => {
            editor.move_to_buffer_end();
        }
        KeyCode::Char('i') => {
            editor.mode = EditorMode::Insert;
            editor.status_message = String::from("Insert mode");
        }
        KeyCode::Char('a') => {
            editor.mode = EditorMode::Insert;
            editor.move_cursor_right();
            editor.status_message = String::from("Insert mode");
        }
        KeyCode::Char('o') => {
            editor.mode = EditorMode::Insert;
            editor.move_to_line_end();
            editor.insert_newline();
            editor.status_message = String::from("Insert mode");
        }
        KeyCode::Char('d') => {
            editor.delete_line();
        }
        KeyCode::Char('y') => {
            editor.yank_line();
        }
        KeyCode::Char('p') => {
            editor.paste_below();
        }
        KeyCode::Char(':') => {
            editor.mode = EditorMode::Command;
            editor.command_buffer.clear();
        }
        KeyCode::Char('/') => {
            editor.mode = EditorMode::Search;
            editor.command_buffer.clear();
        }
        _ => {}
    }
}

fn handle_insert_mode(editor: &mut EditorState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            editor.mode = EditorMode::Normal;
            if editor.cursor_col > 0 && editor.cursor_col >= editor.get_current_line().len() {
                editor.cursor_col -= 1;
            }
            editor.status_message = String::from("Normal mode");
        }
        KeyCode::Char(c) => {
            editor.insert_char(c);
        }
        KeyCode::Backspace => {
            editor.delete_char();
        }
        KeyCode::Enter => {
            editor.insert_newline();
        }
        KeyCode::Left => editor.move_cursor_left(),
        KeyCode::Right => editor.move_cursor_right(),
        KeyCode::Up => editor.move_cursor_up(),
        KeyCode::Down => editor.move_cursor_down(),
        _ => {}
    }
}

fn handle_command_mode(editor: &mut EditorState, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => {
            editor.mode = EditorMode::Normal;
            editor.command_buffer.clear();
            editor.status_message = String::from("Normal mode");
        }
        KeyCode::Char(c) => {
            editor.command_buffer.push(c);
        }
        KeyCode::Backspace => {
            editor.command_buffer.pop();
        }
        KeyCode::Enter => {
            let command = editor.command_buffer.clone();
            editor.execute_command(&command);
            editor.command_buffer.clear();
            editor.mode = EditorMode::Normal;
        }
        _ => {}
    }
}

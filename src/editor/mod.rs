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
        } else if self.mode == EditorMode::Insert && self.cursor_row < self.buffer.len() - 1 {
            // Only wrap to next line in Insert mode
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

    pub fn delete_char_at_cursor(&mut self) {
        let cursor_col = self.cursor_col;
        let line_len = self.get_current_line().len();

        if cursor_col < line_len {
            let line = self.get_current_line_mut();
            line.remove(cursor_col);
            let new_len = line.len();
            self.modified = true;

            // Clamp cursor if we deleted the last character
            if self.mode == EditorMode::Normal && cursor_col >= new_len && new_len > 0 {
                self.cursor_col = new_len - 1;
            }
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
        KeyCode::Char('x') => {
            editor.delete_char_at_cursor();
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

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_editor() -> EditorState {
        let content = "line 1\nline 2\nline 3".to_string();
        EditorState::new("test.txt".to_string(), "/tmp/test.txt".to_string(), content)
    }

    fn create_empty_editor() -> EditorState {
        EditorState::new("empty.txt".to_string(), "/tmp/empty.txt".to_string(), String::new())
    }

    // ===== Cursor Movement Tests =====

    #[test]
    fn test_move_cursor_down() {
        let mut editor = create_test_editor();
        assert_eq!(editor.cursor_row, 0);

        editor.move_cursor_down();
        assert_eq!(editor.cursor_row, 1);

        editor.move_cursor_down();
        assert_eq!(editor.cursor_row, 2);

        // Should not move past last line
        editor.move_cursor_down();
        assert_eq!(editor.cursor_row, 2);
    }

    #[test]
    fn test_move_cursor_up() {
        let mut editor = create_test_editor();
        editor.cursor_row = 2;

        editor.move_cursor_up();
        assert_eq!(editor.cursor_row, 1);

        editor.move_cursor_up();
        assert_eq!(editor.cursor_row, 0);

        // Should not move past first line
        editor.move_cursor_up();
        assert_eq!(editor.cursor_row, 0);
    }

    #[test]
    fn test_move_cursor_right() {
        let mut editor = create_test_editor();
        assert_eq!(editor.cursor_col, 0);

        editor.move_cursor_right();
        assert_eq!(editor.cursor_col, 1);

        // Move to end of line
        for _ in 0..4 {
            editor.move_cursor_right();
        }
        assert_eq!(editor.cursor_col, 5); // "line 1" has 6 chars, max col in normal mode is 5

        // Should not move past end of line in normal mode
        editor.move_cursor_right();
        assert_eq!(editor.cursor_col, 5);
    }

    #[test]
    fn test_move_cursor_right_wraps_to_next_line() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;

        // Move to end of first line
        for _ in 0..6 {
            editor.move_cursor_right();
        }

        // Should wrap to next line
        editor.move_cursor_right();
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn test_move_cursor_left() {
        let mut editor = create_test_editor();
        editor.cursor_col = 3;

        editor.move_cursor_left();
        assert_eq!(editor.cursor_col, 2);

        editor.move_cursor_left();
        assert_eq!(editor.cursor_col, 1);

        editor.move_cursor_left();
        assert_eq!(editor.cursor_col, 0);

        // Should not move past start of line when at row 0
        editor.move_cursor_left();
        assert_eq!(editor.cursor_col, 0);
        assert_eq!(editor.cursor_row, 0);
    }

    #[test]
    fn test_move_cursor_left_wraps_to_previous_line() {
        let mut editor = create_test_editor();
        editor.cursor_row = 1;
        editor.cursor_col = 0;

        editor.move_cursor_left();
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 5); // End of "line 1" in normal mode
    }

    #[test]
    fn test_move_to_line_start() {
        let mut editor = create_test_editor();
        editor.cursor_col = 5;

        editor.move_to_line_start();
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn test_move_to_line_end() {
        let mut editor = create_test_editor();
        editor.cursor_col = 0;

        editor.move_to_line_end();
        assert_eq!(editor.cursor_col, 5); // "line 1" in normal mode

        // In insert mode, should go one past
        editor.mode = EditorMode::Insert;
        editor.cursor_col = 0;
        editor.move_to_line_end();
        assert_eq!(editor.cursor_col, 6);
    }

    #[test]
    fn test_move_to_buffer_start() {
        let mut editor = create_test_editor();
        editor.cursor_row = 2;
        editor.cursor_col = 3;

        editor.move_to_buffer_start();
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn test_move_to_buffer_end() {
        let mut editor = create_test_editor();

        editor.move_to_buffer_end();
        assert_eq!(editor.cursor_row, 2);
        assert_eq!(editor.cursor_col, 5); // End of "line 3"
    }

    // ===== Text Editing Tests =====

    #[test]
    fn test_insert_char() {
        let mut editor = create_empty_editor();
        editor.mode = EditorMode::Insert;

        editor.insert_char('H');
        assert_eq!(editor.buffer[0], "H");
        assert_eq!(editor.cursor_col, 1);
        assert!(editor.modified);

        editor.insert_char('i');
        assert_eq!(editor.buffer[0], "Hi");
        assert_eq!(editor.cursor_col, 2);
    }

    #[test]
    fn test_insert_char_in_middle() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;
        editor.cursor_col = 4; // After "line"

        editor.insert_char('X');
        assert_eq!(editor.buffer[0], "lineX 1");
        assert_eq!(editor.cursor_col, 5);
    }

    #[test]
    fn test_delete_char() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;
        editor.cursor_col = 4;

        editor.delete_char();
        assert_eq!(editor.buffer[0], "lin 1");
        assert_eq!(editor.cursor_col, 3);
        assert!(editor.modified);
    }

    #[test]
    fn test_delete_char_at_line_start_joins_lines() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;
        editor.cursor_row = 1;
        editor.cursor_col = 0;

        editor.delete_char();
        assert_eq!(editor.buffer.len(), 2);
        assert_eq!(editor.buffer[0], "line 1line 2");
        assert_eq!(editor.cursor_row, 0);
        assert_eq!(editor.cursor_col, 6);
    }

    #[test]
    fn test_insert_newline() {
        let mut editor = create_empty_editor();
        editor.mode = EditorMode::Insert;
        editor.insert_char('H');
        editor.insert_char('i');

        editor.insert_newline();
        assert_eq!(editor.buffer.len(), 2);
        assert_eq!(editor.buffer[0], "Hi");
        assert_eq!(editor.buffer[1], "");
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn test_insert_newline_splits_line() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;
        editor.cursor_col = 4; // After "line"

        editor.insert_newline();
        assert_eq!(editor.buffer.len(), 4);
        assert_eq!(editor.buffer[0], "line");
        assert_eq!(editor.buffer[1], " 1");
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 0);
    }

    #[test]
    fn test_delete_char_at_cursor() {
        let mut editor = create_test_editor();
        editor.cursor_col = 2; // At 'n' in "line 1"

        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], "lie 1");
        assert_eq!(editor.cursor_col, 2); // Cursor stays at same position
        assert!(editor.modified);
    }

    #[test]
    fn test_delete_char_at_cursor_end_of_line() {
        let mut editor = create_test_editor();
        editor.cursor_col = 5; // At '1' (last char in "line 1")

        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], "line ");
        assert_eq!(editor.cursor_col, 4); // Cursor clamped to new end
    }

    #[test]
    fn test_delete_char_at_cursor_beyond_line() {
        let mut editor = create_test_editor();
        editor.cursor_col = 10; // Beyond line length

        let original = editor.buffer[0].clone();
        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], original); // No change
        assert!(!editor.modified); // Not modified
    }

    #[test]
    fn test_delete_char_at_cursor_empty_line() {
        let mut editor = create_empty_editor();

        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], ""); // Still empty
        assert!(!editor.modified); // Not modified
    }

    #[test]
    fn test_x_key_deletes_multiple_chars() {
        let mut editor = create_test_editor();
        editor.cursor_col = 0; // At 'l' in "line 1"

        // Delete 'l'
        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], "ine 1");
        assert_eq!(editor.cursor_col, 0);

        // Delete 'i'
        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], "ne 1");
        assert_eq!(editor.cursor_col, 0);

        // Delete 'n'
        editor.delete_char_at_cursor();
        assert_eq!(editor.buffer[0], "e 1");
        assert_eq!(editor.cursor_col, 0);
    }

    // ===== Line Operation Tests =====

    #[test]
    fn test_delete_line() {
        let mut editor = create_test_editor();
        editor.cursor_row = 1;

        editor.delete_line();
        assert_eq!(editor.buffer.len(), 2);
        assert_eq!(editor.buffer[0], "line 1");
        assert_eq!(editor.buffer[1], "line 3");
        assert_eq!(editor.yank_register, vec!["line 2"]);
        assert!(editor.modified);
    }

    #[test]
    fn test_delete_last_line() {
        let mut editor = create_test_editor();
        editor.cursor_row = 2;

        editor.delete_line();
        assert_eq!(editor.buffer.len(), 2);
        assert_eq!(editor.cursor_row, 1); // Should move up
    }

    #[test]
    fn test_delete_only_line() {
        let mut editor = create_empty_editor();
        editor.buffer[0] = "test".to_string();

        editor.delete_line();
        assert_eq!(editor.buffer.len(), 1);
        assert_eq!(editor.buffer[0], "");
        assert_eq!(editor.yank_register, vec!["test"]);
    }

    #[test]
    fn test_yank_line() {
        let mut editor = create_test_editor();
        editor.cursor_row = 1;

        editor.yank_line();
        assert_eq!(editor.yank_register, vec!["line 2"]);
        assert!(!editor.modified); // Yank doesn't modify
    }

    #[test]
    fn test_paste_below() {
        let mut editor = create_test_editor();
        editor.yank_register = vec!["pasted line".to_string()];
        editor.cursor_row = 0;

        editor.paste_below();
        assert_eq!(editor.buffer.len(), 4);
        assert_eq!(editor.buffer[0], "line 1");
        assert_eq!(editor.buffer[1], "pasted line");
        assert_eq!(editor.buffer[2], "line 2");
        assert_eq!(editor.cursor_row, 1);
        assert!(editor.modified);
    }

    #[test]
    fn test_paste_multiple_lines() {
        let mut editor = create_test_editor();
        editor.yank_register = vec!["line A".to_string(), "line B".to_string()];
        editor.cursor_row = 0;

        editor.paste_below();
        assert_eq!(editor.buffer.len(), 5);
        assert_eq!(editor.buffer[1], "line A");
        assert_eq!(editor.buffer[2], "line B");
    }

    // ===== Mode Switching Tests =====

    #[test]
    fn test_mode_starts_in_normal() {
        let editor = create_test_editor();
        assert_eq!(editor.mode, EditorMode::Normal);
    }

    #[test]
    fn test_normal_mode_cursor_clamping() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;
        editor.cursor_col = 6; // At end of line in insert mode

        editor.mode = EditorMode::Normal;
        editor.clamp_cursor();
        assert_eq!(editor.cursor_col, 5); // Should clamp to last char
    }

    #[test]
    fn test_insert_mode_allows_past_end() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;

        editor.move_to_line_end();
        assert_eq!(editor.cursor_col, 6); // Can be at end
    }

    // ===== Command Execution Tests =====

    #[test]
    fn test_command_write() {
        let mut editor = create_test_editor();

        editor.execute_command("w");
        assert_eq!(editor.status_message, "Saving...");
        assert!(!editor.should_quit);
    }

    #[test]
    fn test_command_quit_with_modifications() {
        let mut editor = create_test_editor();
        editor.modified = true;

        editor.execute_command("q");
        assert!(editor.status_message.contains("No write since last change"));
        assert!(!editor.should_quit);
    }

    #[test]
    fn test_command_quit_without_modifications() {
        let mut editor = create_test_editor();
        editor.modified = false;

        editor.execute_command("q");
        assert!(editor.should_quit);
    }

    #[test]
    fn test_command_force_quit() {
        let mut editor = create_test_editor();
        editor.modified = true;

        editor.execute_command("q!");
        assert!(editor.should_quit);
    }

    #[test]
    fn test_command_write_quit() {
        let mut editor = create_test_editor();

        editor.execute_command("wq");
        assert_eq!(editor.status_message, "Saving and quitting...");
    }

    #[test]
    fn test_command_unknown() {
        let mut editor = create_test_editor();

        editor.execute_command("unknown");
        assert!(editor.status_message.contains("Unknown command"));
    }

    // ===== Scroll Logic Tests =====

    #[test]
    fn test_scroll_down_when_cursor_moves_off_screen() {
        let mut editor = create_test_editor();
        editor.scroll_offset = 0;
        editor.cursor_row = 10;

        editor.update_scroll(5); // viewport height = 5

        // Should scroll to keep cursor visible
        assert!(editor.scroll_offset > 0);
        assert!(editor.cursor_row >= editor.scroll_offset);
        assert!(editor.cursor_row < editor.scroll_offset + 5);
    }

    #[test]
    fn test_scroll_up_when_cursor_moves_above_viewport() {
        let mut editor = create_test_editor();
        editor.scroll_offset = 10;
        editor.cursor_row = 5;

        editor.update_scroll(5);

        // Should scroll up to show cursor
        assert!(editor.scroll_offset <= editor.cursor_row);
    }

    // ===== Edge Cases =====

    #[test]
    fn test_empty_file_handling() {
        let editor = create_empty_editor();
        assert_eq!(editor.buffer.len(), 1);
        assert_eq!(editor.buffer[0], "");
    }

    #[test]
    fn test_cursor_clamping_on_shorter_line() {
        let mut editor = create_test_editor();
        editor.buffer = vec![
            "long line here".to_string(),
            "short".to_string(),
        ];
        editor.cursor_col = 10;
        editor.cursor_row = 0;

        editor.move_cursor_down();
        // Cursor should clamp to shorter line
        assert_eq!(editor.cursor_col, 4); // "short" has 5 chars, max col is 4 in normal mode
    }

    #[test]
    fn test_file_paths_stored_correctly() {
        let editor = EditorState::new(
            "test.conf".to_string(),
            "/etc/test.conf".to_string(),
            "config=value".to_string(),
        );

        assert_eq!(editor.filename, "test.conf");
        assert_eq!(editor.remote_path, "/etc/test.conf");
    }

    #[test]
    fn test_modified_flag() {
        let mut editor = create_empty_editor();
        assert!(!editor.modified);

        editor.mode = EditorMode::Insert;
        editor.insert_char('a');
        assert!(editor.modified);
    }

    // ===== Integration-style Tests =====

    #[test]
    fn test_full_editing_workflow() {
        let mut editor = create_empty_editor();
        editor.mode = EditorMode::Insert;

        // Type "Hello"
        for c in "Hello".chars() {
            editor.insert_char(c);
        }

        // New line
        editor.insert_newline();

        // Type "World"
        for c in "World".chars() {
            editor.insert_char(c);
        }

        assert_eq!(editor.buffer.len(), 2);
        assert_eq!(editor.buffer[0], "Hello");
        assert_eq!(editor.buffer[1], "World");
        assert_eq!(editor.cursor_row, 1);
        assert_eq!(editor.cursor_col, 5);
    }

    #[test]
    fn test_delete_and_paste_workflow() {
        let mut editor = create_test_editor();

        // Delete line 2 (cursor at row 0, move down once)
        editor.move_cursor_down();
        editor.delete_line();

        // Paste it at the end
        editor.move_to_buffer_end();
        editor.paste_below();

        assert_eq!(editor.buffer.len(), 3);
        assert_eq!(editor.buffer[0], "line 1");
        assert_eq!(editor.buffer[1], "line 3");
        assert_eq!(editor.buffer[2], "line 2");
    }

    #[test]
    fn test_navigation_and_editing_combo() {
        let mut editor = create_test_editor();
        editor.mode = EditorMode::Insert;

        // Go to end of first line
        editor.move_to_line_end();

        // Add exclamation
        editor.insert_char('!');

        // Go to start of next line
        editor.move_cursor_down();
        editor.move_to_line_start();

        // Insert at beginning
        editor.insert_char('>');

        assert_eq!(editor.buffer[0], "line 1!");
        assert_eq!(editor.buffer[1], ">line 2");
    }

    #[test]
    fn test_multiline_paste() {
        let mut editor = create_empty_editor();

        // Set up multiline yank register
        editor.yank_register = vec![
            "first".to_string(),
            "second".to_string(),
            "third".to_string(),
        ];

        editor.paste_below();

        assert_eq!(editor.buffer.len(), 4); // original empty + 3 pasted
        assert_eq!(editor.buffer[1], "first");
        assert_eq!(editor.buffer[2], "second");
        assert_eq!(editor.buffer[3], "third");
    }
}

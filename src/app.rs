#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified: Option<i64>,
    pub permissions: Option<u32>,
}

pub struct App {
    pub current_path: String,
    pub files: Vec<FileEntry>,
    pub selected_index: usize,
    pub should_quit: bool,
    pub status_message: String,
}

impl App {
    pub fn new() -> Self {
        Self {
            current_path: String::from("/"),
            files: Vec::new(),
            selected_index: 0,
            should_quit: false,
            status_message: String::new(),
        }
    }

    pub fn select_next(&mut self) {
        if !self.files.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.files.len();
        }
    }

    pub fn select_previous(&mut self) {
        if !self.files.is_empty() {
            if self.selected_index == 0 {
                self.selected_index = self.files.len() - 1;
            } else {
                self.selected_index -= 1;
            }
        }
    }

    pub fn get_selected_file(&self) -> Option<&FileEntry> {
        self.files.get(self.selected_index)
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    pub fn set_status(&mut self, message: String) {
        self.status_message = message;
    }
}

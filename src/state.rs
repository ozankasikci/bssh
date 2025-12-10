use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SessionState {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub current_path: String,
    pub selected_index: usize,
}

impl SessionState {
    pub fn new(host: String, port: u16, username: String, current_path: String, selected_index: usize) -> Self {
        Self {
            host,
            port,
            username,
            current_path,
            selected_index,
        }
    }

    fn get_state_file_path(host: &str, port: u16, username: &str) -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
            .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;

        let bssh_dir = config_dir.join("bssh");
        fs::create_dir_all(&bssh_dir)?;

        // Create a unique filename per connection
        let filename = format!("session_{}@{}_{}.json", username, host, port);
        Ok(bssh_dir.join(filename))
    }

    pub fn save(&self) -> Result<()> {
        let state_file = Self::get_state_file_path(&self.host, self.port, &self.username)?;
        let json = serde_json::to_string_pretty(self)?;
        fs::write(state_file, json)?;
        Ok(())
    }

    pub fn load(host: &str, port: u16, username: &str) -> Option<Self> {
        let state_file = Self::get_state_file_path(host, port, username).ok()?;

        if !state_file.exists() {
            return None;
        }

        let json = fs::read_to_string(state_file).ok()?;
        serde_json::from_str(&json).ok()
    }
}

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SavedConnection {
    pub name: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub identity_file: Option<PathBuf>,
}

impl SavedConnection {
    pub fn new(
        name: String,
        host: String,
        port: u16,
        username: String,
        identity_file: Option<PathBuf>,
    ) -> Self {
        Self {
            name,
            host,
            port,
            username,
            identity_file,
        }
    }

    pub fn display_name(&self) -> String {
        format!("{}@{}:{}", self.username, self.host, self.port)
    }
}

fn get_connections_file_path() -> Result<PathBuf> {
    let config_dir = dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .ok_or_else(|| anyhow::anyhow!("Could not find config directory"))?;

    let bssh_dir = config_dir.join("bssh");
    fs::create_dir_all(&bssh_dir)?;

    Ok(bssh_dir.join("connections.json"))
}

pub fn load_connections() -> Result<Vec<SavedConnection>> {
    let path = get_connections_file_path()?;

    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)?;
    let connections: Vec<SavedConnection> = serde_json::from_str(&content)?;
    Ok(connections)
}

pub fn save_connections(connections: &[SavedConnection]) -> Result<()> {
    let path = get_connections_file_path()?;
    let json = serde_json::to_string_pretty(connections)?;
    fs::write(path, json)?;
    Ok(())
}

pub fn add_connection(connection: SavedConnection) -> Result<()> {
    let mut connections = load_connections()?;

    // Remove existing connection with same name if exists
    connections.retain(|c| c.name != connection.name);

    connections.push(connection);
    save_connections(&connections)?;
    Ok(())
}

pub fn remove_connection(name: &str) -> Result<()> {
    let mut connections = load_connections()?;
    connections.retain(|c| c.name != name);
    save_connections(&connections)?;
    Ok(())
}

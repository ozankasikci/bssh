use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
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

    pub fn ssh_command(&self) -> String {
        let mut cmd = format!("ssh -p {} {}@{}", self.port, self.username, self.host);
        if let Some(ref identity_file) = self.identity_file {
            cmd = format!("ssh -i {} -p {} {}@{}", identity_file.display(), self.port, self.username, self.host);
        }
        cmd
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

pub fn update_connection(name: &str, updated: SavedConnection) -> Result<()> {
    let path = get_connections_file_path()?;
    update_connection_in_file(&path, name, updated)
}

fn update_connection_in_file(path: &PathBuf, name: &str, updated: SavedConnection) -> Result<()> {
    let content = fs::read_to_string(path)?;
    let mut connections: Vec<SavedConnection> = serde_json::from_str(&content)?;

    let pos = connections.iter().position(|c| c.name == name);
    match pos {
        Some(idx) => {
            connections[idx] = updated;
            let json = serde_json::to_string_pretty(&connections)?;
            fs::write(path, json)?;
            Ok(())
        }
        None => Err(anyhow::anyhow!("Connection '{}' not found", name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_test_connections(dir: &TempDir) -> PathBuf {
        let bssh_dir = dir.path().join("bssh");
        fs::create_dir_all(&bssh_dir).unwrap();
        bssh_dir.join("connections.json")
    }

    #[test]
    fn test_update_connection_changes_host() {
        let temp_dir = TempDir::new().unwrap();
        let path = setup_test_connections(&temp_dir);

        // Create initial connection
        let initial = SavedConnection::new(
            "myserver".to_string(),
            "old-host.com".to_string(),
            22,
            "user".to_string(),
            None,
        );
        let connections = vec![initial];
        let json = serde_json::to_string_pretty(&connections).unwrap();
        fs::write(&path, json).unwrap();

        // Update the connection with new host
        let updated = SavedConnection::new(
            "myserver".to_string(),
            "new-host.com".to_string(),
            22,
            "user".to_string(),
            None,
        );
        update_connection_in_file(&path, "myserver", updated).unwrap();

        // Verify the update
        let content = fs::read_to_string(&path).unwrap();
        let loaded: Vec<SavedConnection> = serde_json::from_str(&content).unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, "myserver");
        assert_eq!(loaded[0].host, "new-host.com");
    }

    #[test]
    fn test_update_connection_not_found_returns_error() {
        let temp_dir = TempDir::new().unwrap();
        let path = setup_test_connections(&temp_dir);

        // Create a connection with different name
        let conn = SavedConnection::new(
            "other".to_string(),
            "host.com".to_string(),
            22,
            "user".to_string(),
            None,
        );
        let connections = vec![conn];
        let json = serde_json::to_string_pretty(&connections).unwrap();
        fs::write(&path, json).unwrap();

        // Try to update non-existent connection
        let updated = SavedConnection::new(
            "nonexistent".to_string(),
            "new-host.com".to_string(),
            22,
            "user".to_string(),
            None,
        );
        let result = update_connection_in_file(&path, "nonexistent", updated);

        assert!(result.is_err());
    }

    #[test]
    fn test_update_connection_preserves_other_connections() {
        let temp_dir = TempDir::new().unwrap();
        let path = setup_test_connections(&temp_dir);

        // Create multiple connections
        let conn1 = SavedConnection::new("server1".to_string(), "host1.com".to_string(), 22, "user1".to_string(), None);
        let conn2 = SavedConnection::new("server2".to_string(), "host2.com".to_string(), 22, "user2".to_string(), None);
        let conn3 = SavedConnection::new("server3".to_string(), "host3.com".to_string(), 22, "user3".to_string(), None);
        let connections = vec![conn1, conn2, conn3];
        let json = serde_json::to_string_pretty(&connections).unwrap();
        fs::write(&path, json).unwrap();

        // Update only server2
        let updated = SavedConnection::new("server2".to_string(), "updated-host.com".to_string(), 2222, "newuser".to_string(), None);
        update_connection_in_file(&path, "server2", updated).unwrap();

        // Verify all connections
        let content = fs::read_to_string(&path).unwrap();
        let loaded: Vec<SavedConnection> = serde_json::from_str(&content).unwrap();

        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded[0].host, "host1.com");
        assert_eq!(loaded[1].host, "updated-host.com");
        assert_eq!(loaded[1].port, 2222);
        assert_eq!(loaded[1].username, "newuser");
        assert_eq!(loaded[2].host, "host3.com");
    }
}

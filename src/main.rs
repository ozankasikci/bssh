mod app;
mod connection_selector;
mod connections;
mod editor;
mod file_ops;
mod ssh;
mod state;
mod shell;
mod tui;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use connection_selector::ConnectionSelector;
use connections::{add_connection, load_connections, SavedConnection};
use editor::{load_file_content, save_file_content, EditorState, handle_editor_input, render_editor};
use russh_sftp::client::SftpSession;
use ssh::SshClient;
use state::SessionState;
use std::env;
use std::path::PathBuf;
use tui::{handle_input, InputAction, Tui};

#[derive(Parser)]
#[command(name = "bssh")]
#[command(about = "Better SSH - A modern SSH file browser with TUI", long_about = None)]
#[command(version)]
struct Cli {
    /// SSH connection string [user@]host[:port] or saved connection name
    #[arg(value_name = "DESTINATION")]
    destination: Option<String>,

    /// Initial remote directory path
    #[arg(value_name = "PATH")]
    path: Option<String>,

    /// Identity file (private key) for authentication
    #[arg(short = 'i', long = "identity", value_name = "FILE")]
    identity: Option<PathBuf>,

    /// Port to connect to on the remote host
    #[arg(short = 'p', long = "port", value_name = "PORT")]
    port: Option<u16>,

    /// Save this connection for future use
    #[arg(long = "save", value_name = "NAME")]
    save_as: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // If no destination provided, show connection selector
    let (username, host, port, identity_file) = if let Some(dest) = cli.destination {
        // Try to find saved connection by name first
        let saved_connections = load_connections().unwrap_or_default();
        if let Some(conn) = saved_connections.iter().find(|c| c.name == dest) {
            // Use saved connection
            (
                conn.username.clone(),
                conn.host.clone(),
                conn.port,
                conn.identity_file.clone(),
            )
        } else {
            // Parse as connection string
            let (username, host, default_port) = parse_connection_string(&dest)?;
            let port = cli.port.unwrap_or(default_port);
            (username, host, port, cli.identity.clone())
        }
    } else {
        // No destination - show connection selector
        let connections = load_connections().unwrap_or_default();
        let selector = ConnectionSelector::new(connections);

        match selector.run()? {
            Some(conn) => (
                conn.username.clone(),
                conn.host.clone(),
                conn.port,
                conn.identity_file.clone(),
            ),
            None => {
                return Ok(());
            }
        }
    };

    let key_path = identity_file.as_deref();

    println!("Connecting to {}@{}:{}...", username, host, port);
    if let Some(key) = key_path {
        println!("Using identity file: {}", key.display());
    }

    let mut ssh_client = SshClient::connect(&host, port, &username, key_path)
        .await
        .context("Failed to establish SSH connection")?;

    let sftp = ssh_client
        .open_sftp()
        .await
        .context("Failed to open SFTP session")?;

    println!("Connected! Starting TUI...");

    // Save connection if --save flag was provided
    if let Some(save_name) = cli.save_as {
        let connection = SavedConnection::new(
            save_name.clone(),
            host.clone(),
            port,
            username.clone(),
            identity_file.clone(),
        );
        if let Err(e) = add_connection(connection) {
            eprintln!("Warning: Failed to save connection: {}", e);
        } else {
            println!("Connection saved as: {}", save_name);
        }
    }

    // Try to load saved state for this connection
    let (initial_path, initial_index) = if let Some(path_arg) = cli.path.as_deref() {
        // If path was explicitly provided, use it
        (path_arg.to_string(), 0)
    } else if let Some(state) = SessionState::load(&host, port, &username) {
        // Load from saved state
        println!("Restoring previous session: {}", state.current_path);
        (state.current_path, state.selected_index)
    } else {
        // Default to root
        ("/".to_string(), 0)
    };

    run_app(
        ssh_client,
        sftp,
        host.clone(),
        port,
        username.clone(),
        initial_path,
        initial_index
    ).await?;

    Ok(())
}

async fn open_in_editor(
    sftp: &SftpSession,
    remote_path: &str,
    filename: &str,
    tui: &mut Tui,
) -> Result<bool> {
    // Load file content
    let content = load_file_content(sftp, remote_path).await?;
    let mut editor = EditorState::new(filename.to_string(), remote_path.to_string(), content);

    let mut saved = false;
    let mut viewport_height = 20; // Default

    loop {
        tui.terminal.draw(|f| {
            let area = f.area();
            viewport_height = area.height.saturating_sub(2) as usize;
            editor.update_scroll(viewport_height);
            render_editor(f, area, &editor);
        })?;

        if handle_editor_input(&mut editor, viewport_height)? {
            // Check if we need to save
            if editor.status_message == "Saving..." {
                let content = editor.buffer.join("\n");
                save_file_content(sftp, &editor.remote_path, &content).await?;
                editor.modified = false;
                editor.status_message = String::from("Saved");
                saved = true;
            } else if editor.status_message == "Saving and quitting..." {
                let content = editor.buffer.join("\n");
                save_file_content(sftp, &editor.remote_path, &content).await?;
                editor.modified = false;
                saved = true;
                break;
            }
        }

        if editor.should_quit {
            break;
        }
    }

    Ok(saved)
}

async fn run_app(
    _ssh_client: SshClient,
    sftp: SftpSession,
    host: String,
    port: u16,
    username: String,
    initial_path: String,
    initial_index: usize,
) -> Result<()> {
    let connection_string = format!("{}@{}:{}", username, host, port);
    let mut app = App::new(connection_string);
    app.current_path = initial_path;
    app.selected_index = initial_index;

    let mut tui = Tui::new()?;

    app.files = file_ops::list_directory(&sftp, &app.current_path)
        .await
        .unwrap_or_default();

    // Clamp selected index to valid range
    if app.selected_index >= app.files.len() && !app.files.is_empty() {
        app.selected_index = app.files.len() - 1;
    }

    loop {
        tui.draw(&app)?;

        match handle_input()? {
            InputAction::MoveUp => {
                app.select_previous();
            }
            InputAction::MoveDown => {
                app.select_next();
            }
            InputAction::Enter => {
                if let Some(file) = app.get_selected_file() {
                    if file.is_dir {
                        let new_path = if file.name == ".." {
                            get_parent_path(&app.current_path)
                        } else {
                            file.path.clone()
                        };

                        app.current_path = new_path;
                        app.selected_index = 0;

                        match file_ops::list_directory(&sftp, &app.current_path).await {
                            Ok(files) => {
                                app.files = files;
                                app.set_status(String::new());
                            }
                            Err(e) => {
                                app.set_status(format!("Error: {}", e));
                            }
                        }
                    } else {
                        // Save state before opening editor so we can restore position
                        let state = SessionState::new(
                            host.clone(),
                            port,
                            username.clone(),
                            app.current_path.clone(),
                            app.selected_index,
                        );
                        let _ = state.save();

                        // Open file in built-in editor
                        match open_in_editor(&sftp, &file.path, &file.name, &mut tui).await {
                            Ok(saved) => {
                                if saved {
                                    app.set_status(format!("Saved: {}", file.name));
                                } else {
                                    app.set_status(format!("Closed: {}", file.name));
                                }
                            }
                            Err(e) => {
                                app.set_status(format!("Editor error: {}", e));
                            }
                        }
                    }
                }
            }
            InputAction::Download => {
                if let Some(file) = app.get_selected_file() {
                    if !file.is_dir {
                        let local_path = PathBuf::from(&file.name);
                        match file_ops::download_file(&sftp, &file.path, &local_path).await {
                            Ok(_) => {
                                app.set_status(format!("Downloaded: {}", file.name));
                            }
                            Err(e) => {
                                app.set_status(format!("Download failed: {}", e));
                            }
                        }
                    }
                }
            }
            InputAction::Upload => {
                app.set_status("Upload not yet implemented".to_string());
            }
            InputAction::NewDirectory => {
                app.set_status("New directory not yet implemented".to_string());
            }
            InputAction::Rename => {
                app.set_status("Rename not yet implemented".to_string());
            }
            InputAction::Delete => {
                if let Some(file) = app.get_selected_file() {
                    let result = if file.is_dir {
                        file_ops::delete_directory(&sftp, &file.path).await
                    } else {
                        file_ops::delete_file(&sftp, &file.path).await
                    };

                    match result {
                        Ok(_) => {
                            app.set_status(format!("Deleted: {}", file.name));
                            match file_ops::list_directory(&sftp, &app.current_path).await {
                                Ok(files) => {
                                    app.files = files;
                                    if app.selected_index >= app.files.len() && app.selected_index > 0
                                    {
                                        app.selected_index = app.files.len() - 1;
                                    }
                                }
                                Err(e) => {
                                    app.set_status(format!("Error refreshing: {}", e));
                                }
                            }
                        }
                        Err(e) => {
                            app.set_status(format!("Delete failed: {}", e));
                        }
                    }
                }
            }
            InputAction::Execute => {
                app.set_status("Execute not yet implemented".to_string());
            }
            InputAction::ToggleShell => {
                app.set_status("Shell mode not yet implemented".to_string());
            }
            InputAction::Quit => {
                app.quit();
            }
            InputAction::None => {}
        }

        if app.should_quit {
            break;
        }
    }

    // Save state before quitting
    let state = SessionState::new(
        host,
        port,
        username,
        app.current_path,
        app.selected_index,
    );
    let _ = state.save();

    tui.restore()?;
    Ok(())
}

fn parse_connection_string(conn_str: &str) -> Result<(String, String, u16)> {
    let (user_host, port) = if let Some(pos) = conn_str.rfind(':') {
        let port_str = &conn_str[pos + 1..];
        let port = port_str
            .parse::<u16>()
            .context("Invalid port number")?;
        (&conn_str[..pos], port)
    } else {
        (conn_str, 22)
    };

    let (username, host) = if let Some(pos) = user_host.find('@') {
        (user_host[..pos].to_string(), user_host[pos + 1..].to_string())
    } else {
        let current_user = env::var("USER").unwrap_or_else(|_| String::from("root"));
        (current_user, user_host.to_string())
    };

    Ok((username, host, port))
}

fn get_parent_path(path: &str) -> String {
    if path == "/" {
        return String::from("/");
    }

    let path = path.trim_end_matches('/');
    if let Some(pos) = path.rfind('/') {
        if pos == 0 {
            String::from("/")
        } else {
            path[..pos].to_string()
        }
    } else {
        String::from("/")
    }
}

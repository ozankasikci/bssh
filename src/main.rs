mod app;
mod file_ops;
mod ssh;
mod tui;

use anyhow::{Context, Result};
use app::App;
use clap::Parser;
use russh_sftp::client::SftpSession;
use ssh::SshClient;
use std::env;
use std::path::{Path, PathBuf};
use tui::{handle_input, InputAction, Tui};

#[derive(Parser)]
#[command(name = "bssh")]
#[command(about = "Better SSH - A modern SSH file browser with TUI", long_about = None)]
#[command(version)]
struct Cli {
    /// SSH connection string [user@]host[:port]
    #[arg(value_name = "DESTINATION")]
    destination: String,

    /// Initial remote directory path
    #[arg(value_name = "PATH")]
    path: Option<String>,

    /// Identity file (private key) for authentication
    #[arg(short = 'i', long = "identity", value_name = "FILE")]
    identity: Option<PathBuf>,

    /// Port to connect to on the remote host
    #[arg(short = 'p', long = "port", value_name = "PORT")]
    port: Option<u16>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let (username, host, default_port) = parse_connection_string(&cli.destination)?;
    let port = cli.port.unwrap_or(default_port);
    let initial_path = cli.path.as_deref().unwrap_or("/");
    let key_path = cli.identity.as_deref();

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

    run_app(ssh_client, sftp, initial_path.to_string()).await?;

    Ok(())
}

async fn run_app(mut ssh_client: SshClient, sftp: SftpSession, initial_path: String) -> Result<()> {
    let mut app = App::new();
    app.current_path = initial_path;

    let mut tui = Tui::new()?;

    app.files = file_ops::list_directory(&sftp, &app.current_path)
        .await
        .unwrap_or_default();

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
                        // Open file in editor
                        let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vim".to_string());
                        let command = format!("{} '{}'", editor, file.path);

                        // Suspend TUI
                        tui.restore()?;

                        // Execute editor on remote server
                        match ssh_client.execute_interactive(&command).await {
                            Ok(_) => {
                                app.set_status(format!("Closed: {}", file.name));
                            }
                            Err(e) => {
                                app.set_status(format!("Editor error: {}", e));
                            }
                        }

                        // Resume TUI
                        tui = Tui::new()?;
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
            InputAction::Quit => {
                app.quit();
            }
            InputAction::None => {}
        }

        if app.should_quit {
            break;
        }
    }

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

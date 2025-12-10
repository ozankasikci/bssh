use anyhow::{Context, Result};
use russh::client::{self, Handle};
use russh::*;
use russh_keys::key::PublicKey;
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct ConnectionInfo {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub key_path: Option<std::path::PathBuf>,
}

struct Client;

#[async_trait::async_trait]
impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }
}

pub struct SshClient {
    session: Handle<Client>,
    pub connection_info: ConnectionInfo,
}

impl SshClient {
    pub async fn connect(
        host: &str,
        port: u16,
        username: &str,
        key_path: Option<&Path>,
    ) -> Result<Self> {
        let config = client::Config {
            inactivity_timeout: Some(std::time::Duration::from_secs(300)),
            ..<russh::client::Config as Default>::default()
        };

        let sh = Client;
        let mut session = client::connect(Arc::new(config), (host, port), sh)
            .await
            .context("Failed to connect to SSH server")?;

        let key_path_buf = key_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                let home = dirs::home_dir().expect("Could not find home directory");
                home.join(".ssh/id_rsa")
            });

        let key_pair = russh_keys::load_secret_key(&key_path_buf, None)
            .context("Failed to load SSH key")?;

        let auth_res = session
            .authenticate_publickey(username, Arc::new(key_pair))
            .await
            .context("Authentication failed")?;

        if !auth_res {
            anyhow::bail!("Authentication failed");
        }

        let connection_info = ConnectionInfo {
            host: host.to_string(),
            port,
            username: username.to_string(),
            key_path: Some(key_path_buf),
        };

        Ok(Self { session, connection_info })
    }

    pub async fn open_sftp(&mut self) -> Result<SftpSession> {
        let channel = self
            .session
            .channel_open_session()
            .await
            .context("Failed to open channel")?;

        channel
            .request_subsystem(true, "sftp")
            .await
            .context("Failed to request SFTP subsystem")?;

        let sftp = SftpSession::new(channel.into_stream())
            .await
            .context("Failed to create SFTP session")?;

        Ok(sftp)
    }

    pub async fn execute_command(&mut self, command: &str) -> Result<String> {
        let mut channel = self
            .session
            .channel_open_session()
            .await
            .context("Failed to open channel")?;

        channel
            .exec(true, command)
            .await
            .context("Failed to execute command")?;

        let mut output = String::new();
        let mut code = None;

        loop {
            let Some(msg) = channel.wait().await else {
                break;
            };

            match msg {
                ChannelMsg::Data { ref data } => {
                    output.push_str(&String::from_utf8_lossy(data));
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    code = Some(exit_status);
                }
                _ => {}
            }
        }

        if let Some(code) = code {
            if code != 0 {
                anyhow::bail!("Command exited with code {}: {}", code, output);
            }
        }

        Ok(output)
    }

    pub async fn execute_interactive(&mut self, command: &str) -> Result<()> {
        use crossterm::{execute, terminal};

        let mut channel = self
            .session
            .channel_open_session()
            .await
            .context("Failed to open channel")?;

        // Get terminal size
        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        // Request a PTY for interactive programs like vim
        channel
            .request_pty(
                true,
                "xterm-256color",
                cols as u32,
                rows as u32,
                0,
                0,
                &[], // no terminal modes
            )
            .await
            .context("Failed to request PTY")?;

        channel
            .exec(true, command)
            .await
            .context("Failed to execute command")?;

        // Enable raw mode using crossterm (consistent with TUI)
        terminal::enable_raw_mode()?;

        // Channel stream for reading/writing
        let mut stream = channel.into_stream();
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        // Spawn task to forward stdin to remote
        let stdin_task = tokio::spawn(async move {
            let mut stdin = tokio::io::stdin();
            let mut buf = [0u8; 4096];
            loop {
                match stdin.read(&mut buf).await {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if write_half.write_all(&buf[..n]).await.is_err() {
                            break;
                        }
                    }
                }
            }
        });

        // Forward remote output to stdout
        let mut stdout = tokio::io::stdout();
        let mut buf = [0u8; 4096];
        loop {
            match read_half.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    stdout.write_all(&buf[..n]).await?;
                    stdout.flush().await?;
                }
            }
        }

        // Flush output one more time
        stdout.flush().await?;

        // Give stdin task a moment to finish processing
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Abort stdin task
        stdin_task.abort();

        // Flush any pending input BEFORE disabling raw mode
        use crossterm::event::{self, Event};
        while event::poll(std::time::Duration::from_millis(50))? {
            let _ = event::read(); // Consume and discard
        }

        // Now disable raw mode (will be re-enabled by TUI::new())
        terminal::disable_raw_mode()?;

        // One more flush after disabling raw mode
        while event::poll(std::time::Duration::from_millis(50))? {
            let _ = event::read();
        }

        Ok(())
    }

    // Simpler approach: use system ssh command
    pub fn execute_interactive_external(&self, command: &str) -> Result<()> {
        use std::process::Command;

        let mut cmd = Command::new("ssh");

        cmd.arg("-p").arg(self.connection_info.port.to_string());

        if let Some(ref key) = self.connection_info.key_path {
            cmd.arg("-i").arg(key);
        }

        cmd.arg(format!("{}@{}", self.connection_info.username, self.connection_info.host));
        cmd.arg(command);

        let status = cmd.status()?;

        if !status.success() {
            anyhow::bail!("Command failed with exit code: {:?}", status.code());
        }

        Ok(())
    }
}

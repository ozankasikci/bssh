use anyhow::{Context, Result};
use russh::client::{self, Handle};
use russh::*;
use russh_keys::key::PublicKey;
use russh_sftp::client::SftpSession;
use std::path::Path;
use std::sync::Arc;

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

        let key_path = key_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| {
                let home = dirs::home_dir().expect("Could not find home directory");
                home.join(".ssh/id_rsa")
            });

        let key_pair = russh_keys::load_secret_key(&key_path, None)
            .context("Failed to load SSH key")?;

        let auth_res = session
            .authenticate_publickey(username, Arc::new(key_pair))
            .await
            .context("Authentication failed")?;

        if !auth_res {
            anyhow::bail!("Authentication failed");
        }

        Ok(Self { session })
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
}

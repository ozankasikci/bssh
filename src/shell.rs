use anyhow::{Context, Result};
use crossterm::terminal;
use russh::Channel;
use russh::ChannelStream;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct ShellSession {
    channel: Option<Channel<russh::client::Msg>>,
    stream: Option<ChannelStream<russh::client::Msg>>,
    pub is_active: bool,
}

impl ShellSession {
    pub async fn new(
        session: &russh::client::Handle<impl russh::client::Handler>,
        initial_dir: &str,
    ) -> Result<Self> {
        let channel = session
            .channel_open_session()
            .await
            .context("Failed to open shell channel")?;

        let (cols, rows) = terminal::size().unwrap_or((80, 24));

        channel
            .request_pty(
                true,
                "xterm-256color",
                cols as u32,
                rows as u32,
                0,
                0,
                &[],
            )
            .await
            .context("Failed to request PTY")?;

        // Start shell with cd to initial directory
        let shell_cmd = format!("cd {} && exec $SHELL -l", shell_escape(initial_dir));
        channel
            .exec(true, shell_cmd.as_str())
            .await
            .context("Failed to start shell")?;

        let stream = channel.into_stream();

        Ok(Self {
            channel: None,
            stream: Some(stream),
            is_active: true,
        })
    }

    /// Run the shell I/O loop. Returns when user presses Ctrl+s or shell exits.
    /// Returns Ok(true) if user toggled back, Ok(false) if shell exited.
    pub async fn run(&mut self) -> Result<bool> {
        let stream = self.stream.take().context("Stream already consumed")?;
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        let mut stdout = tokio::io::stdout();
        let mut stdin_buf = [0u8; 1024];
        let mut stdout_buf = [0u8; 4096];

        // Use tokio stdin for async reading
        let mut stdin = tokio::io::stdin();

        let result = loop {
            tokio::select! {
                // Read from remote shell, write to local stdout
                result = read_half.read(&mut stdout_buf) => {
                    match result {
                        Ok(0) => {
                            // Shell closed
                            self.is_active = false;
                            break Ok(false);
                        }
                        Ok(n) => {
                            stdout.write_all(&stdout_buf[..n]).await?;
                            stdout.flush().await?;
                        }
                        Err(_) => {
                            self.is_active = false;
                            break Ok(false);
                        }
                    }
                }
                // Read from local stdin, check for Ctrl+s, write to remote
                result = stdin.read(&mut stdin_buf) => {
                    match result {
                        Ok(0) => {
                            // EOF on stdin
                            continue;
                        }
                        Ok(n) => {
                            // Check for Ctrl+s (ASCII 19)
                            if stdin_buf[..n].contains(&19) {
                                // User pressed Ctrl+s, toggle back to browser
                                break Ok(true);
                            }
                            write_half.write_all(&stdin_buf[..n]).await?;
                            write_half.flush().await?;
                        }
                        Err(_) => continue,
                    }
                }
            }
        };

        // If we're toggling back (not exiting), reassemble the stream
        if let Ok(true) = result {
            self.stream = Some(read_half.unsplit(write_half));
        }

        result
    }

    /// Update PTY size after terminal resize
    /// Note: Currently not functional as channel is consumed for stream I/O
    pub async fn update_size(&self) -> Result<()> {
        // TODO: Implement window size updates - requires keeping channel reference
        // For now, initial size is set during PTY request
        Ok(())
    }
}

fn shell_escape(s: &str) -> String {
    // Simple escape: wrap in single quotes, escape existing single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}

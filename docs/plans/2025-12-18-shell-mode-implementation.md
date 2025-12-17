# Shell Mode Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add toggle-able interactive shell mode (Ctrl+s) that persists in background while file browsing.

**Architecture:** Add ShellSession struct to manage persistent PTY channel. Modify main event loop to switch between file browser mode and shell mode. Shell channel coexists with SFTP channel on same SSH connection.

**Tech Stack:** Rust, russh (SSH), crossterm (terminal), ratatui (TUI), tokio (async)

---

### Task 1: Add ToggleShell Input Action

**Files:**
- Modify: `src/tui/mod.rs:192-228`

**Step 1: Add ToggleShell variant to InputAction enum**

In `src/tui/mod.rs`, add `ToggleShell` to the `InputAction` enum:

```rust
pub enum InputAction {
    MoveUp,
    MoveDown,
    Enter,
    Download,
    Upload,
    NewDirectory,
    Rename,
    Delete,
    Execute,
    ToggleShell,  // Add this line
    Quit,
    None,
}
```

**Step 2: Handle Ctrl+s in handle_input function**

In `src/tui/mod.rs`, add the Ctrl+s handler in the `handle_input` match block, before the existing `Ctrl+c` handler:

```rust
KeyCode::Char('s') if key.modifiers.contains(KeyModifiers::CONTROL) => {
    InputAction::ToggleShell
}
```

**Step 3: Build to verify compilation**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles with warning about unused `ToggleShell` variant

**Step 4: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: add ToggleShell input action for Ctrl+s"
```

---

### Task 2: Create ShellSession Module

**Files:**
- Create: `src/shell.rs`
- Modify: `src/main.rs:1-10` (add module declaration)

**Step 1: Create the shell session module**

Create `src/shell.rs` with the ShellSession struct:

```rust
use anyhow::{Context, Result};
use crossterm::terminal;
use russh::Channel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub struct ShellSession {
    channel: Channel<russh::client::Msg>,
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
            .exec(true, &shell_cmd)
            .await
            .context("Failed to start shell")?;

        Ok(Self {
            channel,
            is_active: true,
        })
    }

    /// Run the shell I/O loop. Returns when user presses Ctrl+s or shell exits.
    /// Returns Ok(true) if user toggled back, Ok(false) if shell exited.
    pub async fn run(&mut self) -> Result<bool> {
        let stream = self.channel.make_stream();
        let (mut read_half, mut write_half) = tokio::io::split(stream);

        let mut stdout = tokio::io::stdout();
        let mut stdin_buf = [0u8; 1024];
        let mut stdout_buf = [0u8; 4096];

        // Use tokio stdin for async reading
        let mut stdin = tokio::io::stdin();

        loop {
            tokio::select! {
                // Read from remote shell, write to local stdout
                result = read_half.read(&mut stdout_buf) => {
                    match result {
                        Ok(0) => {
                            // Shell closed
                            self.is_active = false;
                            return Ok(false);
                        }
                        Ok(n) => {
                            stdout.write_all(&stdout_buf[..n]).await?;
                            stdout.flush().await?;
                        }
                        Err(_) => {
                            self.is_active = false;
                            return Ok(false);
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
                                return Ok(true);
                            }
                            write_half.write_all(&stdin_buf[..n]).await?;
                        }
                        Err(_) => continue,
                    }
                }
            }
        }
    }

    /// Update PTY size after terminal resize
    pub async fn update_size(&self) -> Result<()> {
        let (cols, rows) = terminal::size().unwrap_or((80, 24));
        self.channel
            .window_change(cols as u32, rows as u32, 0, 0)
            .await
            .context("Failed to update window size")?;
        Ok(())
    }
}

fn shell_escape(s: &str) -> String {
    // Simple escape: wrap in single quotes, escape existing single quotes
    format!("'{}'", s.replace('\'', "'\\''"))
}
```

**Step 2: Add module declaration to main.rs**

In `src/main.rs`, add the module declaration after line 7 (after `mod state;`):

```rust
mod shell;
```

**Step 3: Build to verify compilation**

Run: `cargo build 2>&1 | head -30`
Expected: Compiles (may have warnings about unused code)

**Step 4: Commit**

```bash
git add src/shell.rs src/main.rs
git commit -m "feat: add ShellSession module for persistent shell"
```

---

### Task 3: Add Shell State Tracking to App

**Files:**
- Modify: `src/app.rs`

**Step 1: Add has_background_shell field to App struct**

In `src/app.rs`, add a field to track if shell is backgrounded:

```rust
pub struct App {
    pub current_path: String,
    pub files: Vec<FileEntry>,
    pub selected_index: usize,
    pub should_quit: bool,
    pub status_message: String,
    pub connection_string: String,
    pub has_background_shell: bool,  // Add this line
}
```

**Step 2: Initialize the field in App::new**

Update the `App::new` function:

```rust
impl App {
    pub fn new(connection_string: String) -> Self {
        Self {
            current_path: String::from("/"),
            files: Vec::new(),
            selected_index: 0,
            should_quit: false,
            status_message: String::new(),
            connection_string,
            has_background_shell: false,  // Add this line
        }
    }
```

**Step 3: Build to verify compilation**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles successfully

**Step 4: Commit**

```bash
git add src/app.rs
git commit -m "feat: add has_background_shell field to App"
```

---

### Task 4: Add Shell Indicator to TUI Header

**Files:**
- Modify: `src/tui/mod.rs:71-88`

**Step 1: Update render_header to show shell indicator**

Replace the `render_header` function in `src/tui/mod.rs`:

```rust
fn render_header(f: &mut Frame, area: Rect, app: &App) {
    let shell_indicator = if app.has_background_shell {
        " [shell]"
    } else {
        ""
    };

    let header = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(&app.connection_string, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(shell_indicator, Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Path: ", Style::default().fg(Color::Yellow)),
            Span::raw(&app.current_path),
        ]),
        Line::from(vec![
            Span::styled("Actions: ", Style::default().fg(Color::Green)),
            Span::raw("Enter=Open  d=Download  Del=Delete  Ctrl+s=Shell  q=Quit"),
        ]),
    ])
    .block(Block::default().borders(Borders::ALL).title("bssh"));

    f.render_widget(header, area);
}
```

**Step 2: Build to verify compilation**

Run: `cargo build 2>&1 | head -20`
Expected: Compiles successfully

**Step 3: Commit**

```bash
git add src/tui/mod.rs
git commit -m "feat: add shell indicator to TUI header"
```

---

### Task 5: Integrate Shell Mode into Main Event Loop

**Files:**
- Modify: `src/main.rs`

**Step 1: Add shell import and update run_app signature**

At the top of `src/main.rs`, add the shell import (around line 18):

```rust
use shell::ShellSession;
```

Update the `run_app` function signature to accept mutable ssh_client:

```rust
async fn run_app(
    mut ssh_client: SshClient,  // Change from _ssh_client to mut ssh_client
    sftp: SftpSession,
    host: String,
    port: u16,
    username: String,
    initial_path: String,
    initial_index: usize,
) -> Result<()> {
```

**Step 2: Add shell session state and helper function**

Add this helper function before `run_app`:

```rust
async fn enter_shell_mode(
    ssh_client: &mut SshClient,
    shell_session: &mut Option<ShellSession>,
    current_path: &str,
    tui: &mut Tui,
) -> Result<bool> {
    // Leave TUI alternate screen for shell
    tui.restore()?;

    // Enable raw mode for shell I/O
    crossterm::terminal::enable_raw_mode()?;

    // Create new shell if none exists
    if shell_session.is_none() {
        *shell_session = Some(ShellSession::new(&ssh_client.session, current_path).await?);
    }

    let session = shell_session.as_mut().unwrap();

    // Update terminal size in case it changed
    session.update_size().await?;

    // Run shell until toggle or exit
    let toggled_back = session.run().await?;

    // Disable raw mode before returning to TUI
    crossterm::terminal::disable_raw_mode()?;

    // Flush any pending input
    while crossterm::event::poll(std::time::Duration::from_millis(50))? {
        let _ = crossterm::event::read();
    }

    if !toggled_back {
        // Shell exited, clear session
        *shell_session = None;
    }

    Ok(toggled_back || shell_session.is_some())
}
```

**Step 3: Update run_app to handle ToggleShell**

In the `run_app` function, add shell session state after creating the app:

```rust
let mut shell_session: Option<ShellSession> = None;
```

Add the ToggleShell handler in the match block (around line 332, before `InputAction::Quit`):

```rust
InputAction::ToggleShell => {
    match enter_shell_mode(
        &mut ssh_client,
        &mut shell_session,
        &app.current_path,
        &mut tui,
    ).await {
        Ok(_) => {
            // Reinitialize TUI after shell mode
            tui = Tui::new()?;
            app.has_background_shell = shell_session.is_some();
            if shell_session.is_none() {
                app.set_status("Shell exited".to_string());
            }
        }
        Err(e) => {
            // Reinitialize TUI on error too
            tui = Tui::new()?;
            app.set_status(format!("Shell error: {}", e));
            shell_session = None;
            app.has_background_shell = false;
        }
    }
}
```

**Step 4: Expose session handle in SshClient**

In `src/ssh/client.rs`, make the session field public:

```rust
pub struct SshClient {
    pub session: Handle<Client>,  // Change from private to pub
    pub connection_info: ConnectionInfo,
}
```

Also need to make Client public or use a different approach. Update the Client struct:

```rust
pub struct Client;
```

And update the Handler impl to be compatible. Actually, we need to use a type alias. Update `src/ssh/client.rs`:

Add after the imports:
```rust
pub type SshSession = Handle<Client>;
```

**Step 5: Update shell.rs to use the correct type**

Update `src/shell.rs` to accept the correct session type:

```rust
use crate::ssh::client::SshSession;

impl ShellSession {
    pub async fn new(
        session: &SshSession,
        initial_dir: &str,
    ) -> Result<Self> {
```

**Step 6: Build to verify compilation**

Run: `cargo build 2>&1`
Expected: Compiles successfully (fix any errors)

**Step 7: Commit**

```bash
git add src/main.rs src/ssh/client.rs src/shell.rs
git commit -m "feat: integrate shell mode into main event loop"
```

---

### Task 6: Test Shell Mode End-to-End

**Step 1: Build release binary**

Run: `cargo build --release`
Expected: Compiles successfully

**Step 2: Manual testing checklist**

Test the following scenarios:
1. Start bssh, connect to a server
2. Press Ctrl+s - should enter shell mode
3. Run commands in shell (ls, pwd, etc.)
4. Press Ctrl+s - should return to file browser with [shell] indicator
5. Press Ctrl+s again - should return to same shell session
6. Type `exit` in shell - should return to browser, [shell] indicator gone
7. Press Ctrl+s again - should start new shell in current directory

**Step 3: Commit any fixes**

```bash
git add -A
git commit -m "fix: shell mode adjustments from testing"
```

---

### Task 7: Update Documentation

**Files:**
- Modify: `README.md`

**Step 1: Add shell mode to keyboard shortcuts table**

Find the keyboard shortcuts table in README.md and add:

```markdown
| `Ctrl+s` | Toggle shell mode |
```

**Step 2: Add shell mode section to Features**

Add to the Features list:

```markdown
- Interactive shell mode - toggle between file browser and full shell with Ctrl+s
```

**Step 3: Add usage section for shell mode**

Add after the Keyboard Shortcuts table:

```markdown
### Shell Mode

Press `Ctrl+s` to toggle into an interactive shell session. The shell starts in your currently browsed directory.

- The shell persists in the background when you toggle back to the file browser
- A `[shell]` indicator appears in the header when a shell session is active
- Press `Ctrl+s` again to return to your shell session
- Type `exit` in the shell to close it and return to browsing
```

**Step 4: Commit**

```bash
git add README.md
git commit -m "docs: add shell mode documentation"
```

---

## Summary

After completing all tasks, you will have:
1. `Ctrl+s` triggers shell mode toggle
2. Shell session persists across toggles
3. `[shell]` indicator shows when shell is backgrounded
4. Shell starts in current browsed directory
5. Documentation updated

Total: 7 tasks with ~20 discrete steps.

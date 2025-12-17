# Shell Mode Design

## Overview

Add the ability to toggle into a full interactive shell session from the file browser, with the shell persisting in the background while browsing.

## User Experience

**Triggering Shell Mode:**
- Press `Ctrl+s` from the file browser to enter shell mode
- If no shell session exists, a new one spawns in the current browsed directory
- If a shell session already exists (backgrounded), switch back to it

**In Shell Mode:**
- File browser UI disappears, replaced by full-screen interactive shell
- Normal SSH shell experience - run commands, see output, use vim, etc.
- Press `Ctrl+s` to toggle back to file browser (shell stays alive in background)

**Shell Lifecycle:**
- Shell persists while backgrounded - toggle back and forth without losing state
- Typing `exit` or shell termination automatically returns to file browser
- New shell session after one ends starts in current browsed directory

**Visual Indicator:**
- When background shell session is active, show `[shell]` in the file browser header

## Technical Implementation

**State Management:**
- Add `Mode::Shell` variant alongside existing file browser mode
- Store shell session handle (PTY/channel) in app state for persistence across toggles
- Track shell session status for UI indicator

**SSH Channel Handling:**
- Open interactive shell channel on existing SSH connection (separate from SFTP channel)
- Request PTY on the channel for terminal emulation
- Shell channel and SFTP channel coexist on same SSH session

**Terminal I/O in Shell Mode:**
- bssh acts as pass-through:
  - Forward keyboard input to remote shell channel
  - Forward shell output to local terminal
- Intercept `Ctrl+s` before forwarding to trigger toggle
- On toggle back, stop forwarding but keep channel open

**Switching Back to File Browser:**
- Restore TUI rendering (ratatui takes over terminal)
- Shell channel remains open and buffered in background

## Edge Cases

- **Connection drops:** Both file browser and shell affected. Show error, return to connection selector.
- **Ctrl+s in shell apps:** Some programs use for flow control (XOFF). Users can run `stty -ixon` if needed.
- **Large output while backgrounded:** Buffer a few KB, drop oldest if exceeded.
- **Terminal resize:** Send window size update to PTY when toggling back to shell mode.

## Directory Behavior

- Shell starts in current browsed directory when first opened
- File browser and shell directories are independent after initial spawn
- No bidirectional sync

## Documentation

Add to keyboard shortcuts table:

| Key | Action |
|-----|--------|
| `Ctrl+s` | Toggle shell mode |

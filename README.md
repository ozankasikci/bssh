# bssh - Better SSH

A modern, user-friendly SSH file browser with a Terminal User Interface (TUI) built in Rust.

## Features

- 📁 Visual file browsing on remote servers
- 🚀 Fast and lightweight
- ⌨️ Keyboard-driven navigation
- 📥 Download files from remote server
- 🗑️ Delete files and directories
- 🔐 SSH key-based authentication
- 📝 ~/.ssh/config integration

## Installation

```bash
cargo build --release
sudo cp target/release/bssh /usr/local/bin/
```

## Usage

```bash
# Connect to a server
bssh user@hostname

# Use custom SSH key (PEM file)
bssh -i ~/.ssh/custom_key.pem user@hostname

# Specify port
bssh -p 2222 user@hostname
# or
bssh user@hostname:2222

# Start in a specific directory
bssh user@hostname /home/user/projects

# Combine options
bssh -i ~/.ssh/mykey.pem -p 2222 user@hostname /var/www

# Use current user
bssh hostname
```

### Command-line Options

```
Usage: bssh [OPTIONS] <DESTINATION> [PATH]

Arguments:
  <DESTINATION>  SSH connection string [user@]host[:port]
  [PATH]         Initial remote directory path

Options:
  -i, --identity <FILE>  Identity file (private key) for authentication
  -p, --port <PORT>      Port to connect to on the remote host
  -h, --help             Print help
  -V, --version          Print version
```

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `↑` / `k` | Move selection up |
| `↓` / `j` | Move selection down |
| `Enter` | Open directory / View file |
| `d` | Download selected file |
| `u` | Upload file (coming soon) |
| `n` | Create new directory (coming soon) |
| `r` | Rename file/directory (coming soon) |
| `Del` / `x` | Delete selected file/directory |
| `e` | Execute command (coming soon) |
| `q` / `Ctrl+C` | Quit |

## Authentication

bssh uses SSH key-based authentication. By default, it looks for your SSH key at `~/.ssh/id_rsa`.

### Using a Custom Key

You can specify a custom identity file (PEM key) using the `-i` flag:

```bash
bssh -i ~/.ssh/custom_key.pem user@hostname
```

Alternatively, configure it in your `~/.ssh/config`:

```
Host myserver
    HostName example.com
    User myuser
    IdentityFile ~/.ssh/custom_key
```

## Technical Stack

- **SSH Client**: [russh](https://github.com/Eugeny/russh) - Pure Rust SSH implementation
- **SFTP**: [russh-sftp](https://github.com/AspectUnk/russh-sftp) - SFTP subsystem for russh
- **TUI Framework**: [ratatui](https://github.com/ratatui/ratatui) - Terminal UI library
- **Terminal Backend**: [crossterm](https://github.com/crossterm-rs/crossterm) - Cross-platform terminal manipulation
- **Async Runtime**: [tokio](https://tokio.rs) - Asynchronous runtime

## Development Status

Current features:
- ✅ File browsing
- ✅ Directory navigation
- ✅ File download
- ✅ File/directory deletion
- ✅ SSH key authentication
- ✅ Custom identity file support (-i flag)
- ✅ Custom port support (-p flag)

Coming soon:
- ⏳ File upload
- ⏳ File rename
- ⏳ Create directories
- ⏳ Execute remote commands
- ⏳ File viewing
- ⏳ Search functionality
- ⏳ Multiple file selection
- ⏳ File permissions editing

## License

MIT

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

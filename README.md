# edit — terminal text editor in Rust

A modal terminal text editor built on raw ANSI escape sequences, written in Rust. Supports syntax highlighting, tabs, file browser, themes, and Vim‑like command mode.

## Features

- Syntax highlighting: keywords, strings, numbers, comments (`//`, `/* */`), block strings (`"""`), attributes (`#[...]`)
- Multiple open files via tabs + built-in file browser
- Search with match counter (`Ctrl+F`, navigate with Up/Down)
- Undo / redo history
- Clipboard: copy, cut, paste (with system clipboard via `xclip`/`wl-copy`/`xsel`)
- Mouse support: click to position cursor, scroll wheel
- Command mode (`Esc`): `:w`, `:q`, `:wq`, `:numlist`, `:set`, `:theme`, `:help`
- Configurable key bindings
- Themes (create/edit with `:mktheme` / `:edtheme`)
- Dynamic line numbers, scrollbar, line selection with Shift+arrows

## Installation

Requires Rust 1.70+ and a C compiler:

```bash
# Install build tools (Debian/Ubuntu)
sudo apt-get install -y build-essential
sudo apt install xclip

# Build
git clone <repo> && cd editor
cargo build --release

# Install to PATH
cargo run --release
cp target/release/edit ~/.cargo/bin/
```

## Usage

```bash
edit <file>           # Open file (creates if doesn't exist)
edit                  # Start with empty buffer
edit dir/             # Start with file browser in dir
```

## Keybindings

| Key | Action |
|-----|--------|
| `Ctrl+Q` | Quit |
| `Ctrl+S` | Save |
| `Ctrl+Z` | Undo |
| `Ctrl+Y` | Redo |
| `Ctrl+F` | Search |
| `Ctrl+C` | Copy selection |
| `Ctrl+X` | Cut selection |
| `Ctrl+V` | Paste |
| `Ctrl+A` | Select all |
| `Ctrl+D` | Delete line |
| `Ctrl+K` | Clear buffer |
| `Ctrl+Home` | Go to file start |
| `Ctrl+End` | Go to file end |
| `Ctrl+Backspace` | Delete word |
| `Shift+←/→/↑/↓` | Select text |
| `Home` / `End` | Line start / end |
| `PageUp` / `PageDown` | Page up / down |
| `Tab` | Next tab |
| `Shift+Tab` | Previous tab |
| `Esc` | Enter command mode |

### Command mode (`Esc`)

| Command | Action |
|---------|--------|
| `:q` | Quit |
| `:w` | Save |
| `:wq` | Save and quit |
| `:numlist` | Toggle line numbers |
| `:set <key> <value>` | Set config option |
| `:theme <name>` | Apply theme |
| `:mktheme <name>` | Create / edit theme |
| `:edtheme <name>` | Open theme for editing |
| `:help` | Show help |
| `:<number>` | Go to line |

### Search mode (`Ctrl+F`)

| Key | Action |
|-----|--------|
| `Up` | Previous match |
| `Down` | Next match |
| `Esc` | Cancel search |
| `Enter` | Confirm and exit search |

## Configuration

Settings are stored in `~/.config/edit/config.toml`. 
Key bindings can be remapped:

```
:set bind_save Ctrl+S
:set bind_quit Ctrl+Q
```

# QueryX

A fast, light desktop SQL client for **PostgreSQL** and **SQLite**, written in Rust on [GPUI](https://www.gpui.rs/) (Zed's UI framework). The aim is DataGrip's power with less weight: no JVM, about 30 MB of private memory, no CPU use while idle.

![QueryX showing a 1M-row table, the explorer and the query log](.github/screenshot.png)

## Features

- **Explorer**: a lazy, virtualized tree of connections, databases, schemas and objects, with a filter, refresh and grouped context menus for create, modify, drop and DDL.
- **Connection identity**: each connection has its own color, which runs from the tree dot to the tab stripe, the status bar and the grid header. Production connections are tinted and badged `PROD` everywhere, and writes to them ask for confirmation.
- **Data grid**: virtualized rows and columns (1M rows × 50 columns scrolls at display rate), with selection, copy, column resize and inline cell editing that you apply as one change set.
- **Table tabs**: data, structure and DDL sub-tabs. `WHERE` and `ORDER BY` boxes have SQL completion. Paging is built in.
- **Query console**: a tree-sitter SQL editor. Run the statement under the cursor or the whole script, cancel a running query, and manage transactions. Multiple result sets are supported, and autocomplete draws on the live schema.
- **Bottom panel**: a log of every executed SQL statement (connection, duration, rows, source), filterable, plus messages.
- **Export**: results to CSV, TSV, JSON, SQL `INSERT` or Markdown, to a file or the clipboard.
- **Keyboard-first**: command palette, quick open for any object, a cheat sheet and a rebindable keymap. Every action is also reachable with the mouse.
- **Persistence**: workspaces, connections, layout and open tabs survive a restart. Passwords live only in the OS keychain.
- **Themes**: dark, light or follow the system, plus your own JSON themes, which reload live.
- **TLS** for Postgres via rustls (`prefer` and `require` SSL modes).

## Install

There are no release builds yet. Build from source with Rust (the version is pinned in `rust-toolchain.toml`, and rustup picks it up):

```sh
cargo build --release
./target/release/queryx          # your connections
./target/release/queryx --demo   # built-in sample workspace, no database needed
```

On Linux you need the GPUI system libraries. On Debian/Ubuntu:

```sh
sudo apt install pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
  libx11-xcb-dev libvulkan-dev libfontconfig-dev libfreetype-dev libdbus-1-dev
```

## Keyboard shortcuts

| Action | Keys |
|---|---|
| Command palette | `Ctrl+Shift+P` |
| Quick open | `Ctrl+P` |
| New console | `Ctrl+Shift+N` |
| Run statement / run script | `Ctrl+Enter` / `Ctrl+Shift+Enter` |
| Cancel query | `Esc` |
| Toggle explorer / bottom panel | `Ctrl+B` / `Ctrl+J` |
| Close / reopen tab | `Ctrl+W` / `Ctrl+Shift+T` |
| Cheat sheet | `Ctrl+K Ctrl+S` |
| Cycle theme | `Ctrl+K Ctrl+T` |

Rebind any of them in `keymap.toml` in the config directory.

## Configuration

Config is stored in `$XDG_CONFIG_HOME/queryx` (default `~/.config/queryx`) on Linux, `~/Library/Application Support/queryx` on macOS and `%APPDATA%\queryx` on Windows:

- `config.toml`: settings, theme and layout
- `workspaces.toml`: workspaces and connections (passwords are not stored here)
- `keymap.toml`: key overrides
- `themes/*.json`: user themes

## Development

```sh
cargo test                              # unit and UI tests
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

The Postgres integration tests need a test server. They are skipped when `QUERYX_TEST_PG` is unset:

```sh
podman-compose up -d                    # or: docker compose up -d
export QUERYX_TEST_PG=postgres://queryx:queryx@localhost:55432/queryx
cargo test
```

`scripts/perf.sh` checks the performance budgets (cold start, idle memory, a 100k-row result, binary size). The vocabulary lives in [`GLOSSARY.md`](GLOSSARY.md).

## License

QueryX is licensed under the [GNU General Public License v3.0 or later](LICENSE). The bundled Inter and JetBrains Mono fonts are under the SIL Open Font License (see `assets/fonts/`).

# TableX

TableX is a desktop SQL client for PostgreSQL and SQLite. It's written in Rust on [GPUI](https://www.gpui.rs/).

It uses about 30 MB of RAM and sits at 0% CPU when you're not touching it. Run it with `--demo` to try it on a built-in sample workspace without any database.

TableX is early (0.1.6). Expect breaking changes, and keep backups before pointing it at anything you care about. It's developed and tested on Linux. macOS and Windows builds pass CI but get much less use.

![TableX showing a 1M-row table, the explorer and the query log](.github/screenshot.png)

## What it does

- The explorer on the left is a lazy tree of connections, databases, schemas and objects. You can filter it, refresh it, and right-click things to create, modify, drop or view DDL.
- Every connection gets a color, and you'll see it on the tree dot, the tab stripe, the status bar and the grid header. Production connections are tinted and marked `PROD` everywhere, and TableX asks before writing to them.
- The data grid virtualizes rows and columns, so 1M rows × 50 columns scrolls at full frame rate. You can select, copy, resize columns and edit cells inline, then apply the edits together.
- Opening a table gives you data, structure and DDL tabs. The `WHERE` and `ORDER BY` boxes autocomplete SQL, and results are paged.
- The query console uses tree-sitter for SQL. Run the statement under the cursor or the whole script, cancel long queries, and manage transactions yourself. A script can return several result sets, and completion uses the live schema.
- The bottom panel logs every statement that ran, with connection, duration, row count and where it came from. It also shows messages, and you can filter it.
- Results export to CSV, TSV, JSON, SQL `INSERT` or Markdown, either to a file or the clipboard.
- Almost everything has a key: command palette, quick open for any object, a cheat sheet, and a keymap you can change. The mouse still works for all of it.
- Workspaces, connections, layout and open tabs come back after a restart. Passwords go in the OS keychain and nowhere else.
- Dark, light or follow-the-system themes. You can add your own JSON themes, and they reload as you edit them.
- Postgres connections support TLS through rustls (`prefer` and `require`).

## Install

On Linux (x86_64), this installs the latest release to `~/.local` with a desktop entry and icons:

```sh
curl -fsSL https://github.com/0xSatbeer/TableX/releases/latest/download/install.sh | sh
```

To build from source instead, you need [rustup](https://rustup.rs/). The project pins Rust 1.99.0 in `rust-toolchain.toml`, and rustup downloads it on the first build. That first build takes a while.

```sh
git clone https://github.com/0xSatbeer/TableX.git
cd TableX
cargo build --release
./target/release/tablex          # your connections
./target/release/tablex --demo   # sample workspace, no database needed
```

On Linux, GPUI needs some system libraries. On Debian/Ubuntu:

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

To change any of these, edit `keymap.toml` in the config directory.

## Configuration

The config directory is `$XDG_CONFIG_HOME/tablex` (usually `~/.config/tablex`) on Linux, `~/Library/Application Support/tablex` on macOS, and `%APPDATA%\tablex` on Windows. Inside it:

- `config.toml` has settings, theme and layout
- `workspaces.toml` has workspaces and connections, but not passwords
- `keymap.toml` has your key overrides
- `themes/*.json` are your themes

## Development

```sh
cargo test --locked                     # unit and UI tests
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

These are the same checks CI runs. `cargo test` works without Postgres; the Postgres integration tests are skipped if `TABLEX_TEST_PG` isn't set. To run them, start the test server from `compose.yaml` (it listens on port 55432 so it won't clash with a local Postgres):

```sh
podman-compose up -d                    # or: docker compose up -d
export TABLEX_TEST_PG=postgres://tablex:tablex@localhost:55432/tablex
cargo test
```

`scripts/perf.sh` checks the performance budgets: cold start, idle memory, a 100k-row result, grid scroll frame rate and binary size. It needs `podman-compose` (not Docker), `sqlite3`, `jq` and a display. Terms used in the code are defined in [`GLOSSARY.md`](GLOSSARY.md).

## Releasing

1. Bump `version` in `Cargo.toml`, add a `<release>` to `assets/icons/app/io.github.TableX.metainfo.xml`, and rename `## [Unreleased]` in `CHANGELOG.md` to the new version with today's date.
2. Commit, then tag and push: `git tag v0.2.0 && git push origin main v0.2.0`.

CI refuses a tag that disagrees with any of the three, builds every platform, and publishes the release with the changelog section as its notes. A tag with a suffix such as `v0.2.0-rc.1` becomes a prerelease. Downloads can be checked with `sha256sum -c SHA256SUMS --ignore-missing` or `gh attestation verify <file> --repo 0xSatbeer/TableX`.

## License

GPL v3.0 or later, see [LICENSE](LICENSE). The bundled Inter and JetBrains Mono fonts are under the SIL Open Font License (see `assets/fonts/`).

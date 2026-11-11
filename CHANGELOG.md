# Changelog

All notable changes to TableX are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Each release's GitHub notes are its section below.

## [Unreleased]

## [0.1.6] - 2026-11-11

### Added
- Click a column header to sort by it; the header shows an arrow for the sorted column.
- Cells edit multi-line text: Enter commits, Shift+Enter adds a line break. Line breaks show as ↵ in the grid.
- A Stop button cancels a page load that is still running and keeps the rows already shown.
- Completion items show the explorer's icon for their kind, a column's table and type, and an object's schema.

### Changed
- Reloading a table dims the current rows instead of blanking the table.
- Completion follows the SQL grammar: after a column or value it offers operators, and after `=` or `LIKE` it waits until you start typing a name.
- A connection added from the dialog connects right away.

### Fixed
- Grid rows update their hover highlight as the mouse moves between rows.

## [0.1.5] - 2026-11-04

### Added
- A welcome tour on first run: pick a theme and interface size, learn the main shortcuts and add your first connection. Run it again any time with `Welcome` from the command palette.

### Changed
- With no connections, the workspace shows an Add connection prompt and hides the explorer and bottom panel until there is something to show.

## [0.1.4] - 2026-11-03

### Changed
- The app id is now `io.github.TableX` and the repository moved to [0xSatbeer/TableX](https://github.com/0xSatbeer/TableX); old links redirect. `install.sh` removes the old QueryX desktop entry and icons.
- Everything named `queryx` is now `tablex`: the config directory (`~/.config/tablex`), the keychain service, keymap actions (`tablex::CloseTab`) and the `TABLEX_*` environment variables. Nothing is carried over: re-add connections and passwords, or move `~/.config/queryx` to `~/.config/tablex` and rename `queryx::` to `tablex::` in `keymap.toml`.

## [0.1.3] - 2026-11-03

### Changed
- QueryX is now TableX. The binary is `tablex`, release files are `tablex-*`, and the .deb replaces the `queryx` package. Your connections, saved passwords, keymap and themes carry over unchanged.

### Fixed
- Scrolling the data grid stutters less: a scrolled frame costs about half what it did (14 ms to 8 ms on a 1M-row table), so it holds 60 fps on slower machines.
- Windows release builds no longer open a console window behind the app.

### Security
- Releases ship a `SHA256SUMS` file and GitHub build provenance; `install.sh` verifies the checksum before installing.

## [0.1.2] - 2026-11-02

### Added
- Settings dialog and Darcula theme.
- Custom title bar with desktop-side window buttons and a qualified window title.
- Explorer toolbar and a funnel toggle for the filter row; filter boxes get a clear button.
- Key icons on console result headers and a read-only query results toolbar.
- Type icons in grid headers and cell edit revert.
- Logs panel as console output; tab titles name the connection when they clash.
- First-run empty states and a centered default window.
- Linux tarball with desktop file and icons, and a `curl | sh` install script.

### Changed
- The Messages tab is gone; its output lives in the logs panel.

### Fixed
- Grid, toolbar, panel and table viewer backgrounds and empty-state text colors.
- Workspace menu icon and dropdown caret.

## [0.1.1] - 2026-10-29

### Added
- `.deb` and AppImage packages, with the app id `io.github._0xSatbeer.QueryX` and AppStream metainfo.

## [0.1.0] - 2026-10-29

First public release.

[Unreleased]: https://github.com/0xSatbeer/TableX/compare/v0.1.6...HEAD
[0.1.6]: https://github.com/0xSatbeer/TableX/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/0xSatbeer/TableX/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/0xSatbeer/TableX/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/0xSatbeer/TableX/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/0xSatbeer/TableX/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/0xSatbeer/TableX/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/0xSatbeer/TableX/releases/tag/v0.1.0

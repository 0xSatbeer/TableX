# Changelog

All notable changes to TableX are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Each release's GitHub notes are its section below.

## [Unreleased]

## [0.1.3] - 2026-11-03

### Changed
- QueryX is now TableX. The binary is `tablex`, release files are `tablex-*`, and the .deb replaces the `queryx` package. Your connections, saved passwords, keymap and themes carry over unchanged.

### Fixed
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

[Unreleased]: https://github.com/0xSatbeer/QueryX/compare/v0.1.3...HEAD
[0.1.3]: https://github.com/0xSatbeer/QueryX/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/0xSatbeer/QueryX/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/0xSatbeer/QueryX/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/0xSatbeer/QueryX/releases/tag/v0.1.0

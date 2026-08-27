# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- A bare `curl | bash` install writes `$HOME/.local/bin/macstrap` and exits. Extra arguments after `bash -s --` are still passed through to the installed binary.

## [0.2.0] - 2026-08-27

### Added

- Optional bundled catalogs (node, python, and rust essentials and full sets) composable in the picker.
- Catalog metadata from YAML (`title`, optional `description`) with `brew info` fallback for formulae and casks.
- Installed and available version columns in the tool picker.
- Catalog origin labels (`builtin` / `local`) on the catalogs screen.
- Preselected installed tools. Deselecting an installed tool uninstalls it on apply.
- `o` toggle to show every installed Homebrew package, including packages outside loaded catalogs.
- Custom local catalog files via `macstrap catalog create` or `n` in the catalog screen.
- `mas` to the always-on CLI essentials set.
- Auto-install of all CLI essentials after platform essentials and before the picker.
- Catalog selection persistence in `~/.config/macstrap/config.yml`.
- Color-coded tool rows. Green for installed catalog tools, grey for installed tools outside loaded catalogs.
- Styled startup, apply, and quit output with section headers, icons, and terminal colors.

### Changed

- Picker returns after apply so you can keep installing or uninstalling without restarting.
- Tool list sorted by title. Description columns clip cleanly on narrow terminals.
- Essentials row label reads `Command Line Tools` instead of `CLT`.
- Quit shows a styled `Goodbye 👋` instead of `aborted`.

### Fixed

- Removed stray packages from default bundled catalog lists.

## [0.1.0] - 2026-08-26

### Added

- Bootstrap Command Line Tools, Homebrew, and oh-my-zsh.
- Ratatui catalog picker for formulae, casks, and Mac App Store apps.
- Brewfile v1 parsing merged into picker selection.
- Bundled `cli-essentials.yml` catalog (git, gh, fd, fzf, jq).
- curl installer script and GitHub Actions release workflow for macOS arm64 and amd64.

[Unreleased]: https://github.com/b00gizm/macstrap/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/b00gizm/macstrap/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/b00gizm/macstrap/releases/tag/v0.1.0

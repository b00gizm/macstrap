# Bootstrap a Mac for development

Install Xcode Command Line Tools, Homebrew, and oh-my-zsh. Then pick extra formulae, casks, and Mac App Store apps in a terminal UI.

If `$HOME/Brewfile` exists, those rows start checked. You can toggle any row, select all, or select none. A second run skips what is already present.

The crate name in `Cargo.toml` is a working title. Rust reads it through `CARGO_PKG_NAME`. The installer script has `NAME` and `REPO` at the top. Change those when you rename the project.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/b00gizm/macstrap/main/scripts/install.sh | bash
```

The script downloads the latest release binary for your Mac, checks the SHA-256, and writes it to `$HOME/.local/bin`. It needs no sudo.

Until a release exists, build from this repo:

```bash
cargo run
```

## Usage

```bash
cargo run -- --help
```

`--yes` applies `cli-essentials.yml` plus the Brewfile selection, without the picker. Optional catalogs are a picker action.

`--brewfile PATH` reads that file instead of `$HOME/Brewfile` or `$HOME/.Brewfile`.

`--essentials-only` stops after Command Line Tools, Homebrew, and oh-my-zsh.

The picker uses `space` to toggle, `a` for all, `n` for none, `/` to filter, `c` for catalogs, and `enter` to confirm. `q` or `ctrl+c` aborts. Already installed rows stay visible and are skipped on apply.

`c` opens the catalog list. Each row shows an origin and a description. Bundled files are `builtin`. `cli-essentials.yml` is always loaded. Space loads or unloads the others. The tool list updates as soon as a file is loaded or unloaded.

Descriptions are optional. An empty description stays blank. A formula or cask without one uses `brew info`. That same call fills the installed and available version columns.

Deselect does not uninstall. The tool does not change your login shell unless it is still bash.

## Brewfile

v1 reads `brew "name"`, `cask "name"`, and `mas "Title", id: 123`. Other lines are skipped and counted.

Curated lists live in `catalogs/`. `cli-essentials.yml` is always on. The others are optional:

- `node-essentials.yml` and `node-full.yml`
- `python-essentials.yml` and `python-full.yml`
- `rust-essentials.yml` and `rust-full.yml`

A full file is a superset of the matching essentials file. Names that appear only in the Brewfile still show up in the picker.

Each catalog file has a `title`, an optional `description`, and a `packages` list. Origin is not in the file. The app stamps `builtin` for bundled catalogs.

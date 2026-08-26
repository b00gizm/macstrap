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

`--yes` applies the Brewfile selection without the picker.

`--brewfile PATH` reads that file instead of `$HOME/Brewfile` or `$HOME/.Brewfile`.

`--essentials-only` stops after Command Line Tools, Homebrew, and oh-my-zsh.

The picker uses `space` to toggle, `a` for all, `n` for none, `/` to filter, and `enter` to confirm. `q` or `ctrl+c` aborts. Already installed rows stay visible and are skipped on apply.

Deselect does not uninstall. The tool does not change your login shell unless it is still bash.

## Brewfile

v1 reads `brew "name"`, `cask "name"`, and `mas "Title", id: 123`. Other lines are skipped and counted.

The curated list lives in `catalog.yaml`. Names that appear only in the Brewfile still show up in the picker.

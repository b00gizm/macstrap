#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crossterm::style::{Attribute, Color, Print, ResetColor, SetAttribute, SetForegroundColor};
use crossterm::execute;

use crate::catalog::{self, BrewFacts, Kind, Observed, Package, PkgId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Satisfied,
    Applied,
    Removed,
    Failed,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Applied => "applied",
            Self::Removed => "removed",
            Self::Failed => "failed",
        }
    }
}

#[derive(Debug)]
pub enum Error {
    Message(String),
    Io(io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(s) => f.write_str(s),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

pub trait Host {
    fn has_clt(&self) -> bool;
    fn install_clt(&self) -> Result<(), Error>;
    fn brew_bin(&self) -> Option<PathBuf>;
    fn install_brew(&self) -> Result<(), Error>;
    fn has_omz(&self) -> bool;
    fn install_omz(&self) -> Result<(), Error>;
    fn installed(&self) -> Result<Observed, Error>;
    fn install(&self, pkg: &Package) -> Result<(), Error>;
    fn uninstall(&self, pkg: &Package) -> Result<(), Error>;
    fn brew_facts(&self, packages: &[Package]) -> Result<HashMap<PkgId, BrewFacts>, Error>;
}

pub fn ensure_clt(host: &impl Host) -> Outcome {
    if host.has_clt() {
        return Outcome::Satisfied;
    }
    match host.install_clt() {
        Ok(()) => Outcome::Applied,
        Err(_) => Outcome::Failed,
    }
}

pub fn ensure_brew(host: &impl Host) -> Outcome {
    if host.brew_bin().is_some() {
        return Outcome::Satisfied;
    }
    match host.install_brew() {
        Ok(()) => Outcome::Applied,
        Err(_) => Outcome::Failed,
    }
}

pub fn ensure_omz(host: &impl Host) -> Outcome {
    if host.has_omz() {
        return Outcome::Satisfied;
    }
    match host.install_omz() {
        Ok(()) => Outcome::Applied,
        Err(_) => Outcome::Failed,
    }
}

pub struct Essentials {
    pub clt: Outcome,
    pub brew: Outcome,
    pub omz: Outcome,
}

pub fn ensure_essentials(host: &impl Host) -> Essentials {
    Essentials {
        clt: ensure_clt(host),
        brew: ensure_brew(host),
        omz: ensure_omz(host),
    }
}

pub fn ensure_cli_essentials(host: &impl Host) -> Result<(), Error> {
    let loaded = catalog::default_loaded();
    let packages = catalog::compose(&loaded).map_err(Error::Message)?;
    let mut observed = host.installed()?;
    let mut failed = 0;
    for pkg in &packages {
        let outcome = ensure_package(host, pkg, &observed);
        print_outcome(&pkg.name, outcome);
        match outcome {
            Outcome::Applied | Outcome::Satisfied => {
                observed.insert(pkg.id.clone());
            }
            Outcome::Failed => failed += 1,
            Outcome::Removed => {}
        }
    }
    if failed > 0 {
        return Err(Error::Message(format!("{failed} cli essential(s) failed")));
    }
    Ok(())
}

pub fn ensure_package(host: &impl Host, pkg: &Package, observed: &Observed) -> Outcome {
    if observed.contains(&pkg.id) {
        return Outcome::Satisfied;
    }
    match host.install(pkg) {
        Ok(()) => Outcome::Applied,
        Err(_) => Outcome::Failed,
    }
}

pub fn remove_package(host: &impl Host, pkg: &Package, observed: &Observed) -> Outcome {
    if !observed.contains(&pkg.id) {
        return Outcome::Satisfied;
    }
    match host.uninstall(pkg) {
        Ok(()) => Outcome::Removed,
        Err(_) => Outcome::Failed,
    }
}

pub struct Live;

impl Host for Live {
    fn has_clt(&self) -> bool {
        Command::new("xcode-select")
            .arg("-p")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    fn install_clt(&self) -> Result<(), Error> {
        println!("A macOS dialog will ask you to install the Command Line Tools.");
        let _ = Command::new("xcode-select").arg("--install").status()?;
        for _ in 0..180 {
            if self.has_clt() {
                return Ok(());
            }
            thread::sleep(Duration::from_secs(2));
        }
        Err(Error::Message(
            "Command Line Tools did not finish installing".into(),
        ))
    }

    fn brew_bin(&self) -> Option<PathBuf> {
        find_brew()
    }

    fn install_brew(&self) -> Result<(), Error> {
        let status = Command::new("/bin/bash")
            .arg("-c")
            .arg("curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh | bash")
            .env("NONINTERACTIVE", "1")
            .status()?;
        if !status.success() {
            return Err(Error::Message("Homebrew installer failed".into()));
        }
        if find_brew().is_none() {
            return Err(Error::Message(
                "Homebrew installed but brew was not found".into(),
            ));
        }
        Ok(())
    }

    fn has_omz(&self) -> bool {
        home_dir()
            .map(|h| h.join(".oh-my-zsh").is_dir())
            .unwrap_or(false)
    }

    fn install_omz(&self) -> Result<(), Error> {
        let status = Command::new("/bin/sh")
            .arg("-c")
            .arg("curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh | sh")
            .env("RUNZSH", "no")
            .env("CHSH", "no")
            .env("KEEP_ZSHRC", "yes")
            .status()?;
        if !status.success() {
            return Err(Error::Message("oh-my-zsh installer failed".into()));
        }
        Ok(())
    }

    fn installed(&self) -> Result<Observed, Error> {
        let mut out = Observed::new();
        if let Some(brew) = find_brew() {
            for name in brew_list(&brew, "--formula")? {
                out.insert(PkgId::new(Kind::Formula, &name, None));
            }
            for name in brew_list(&brew, "--cask")? {
                out.insert(PkgId::new(Kind::Cask, &name, None));
            }
        }
        if let Some(mas) = find_mas() {
            for (id, title) in mas_list(&mas)? {
                out.insert(PkgId::new(Kind::Mas, &title, Some(id)));
            }
        }
        Ok(out)
    }

    fn install(&self, pkg: &Package) -> Result<(), Error> {
        match pkg.kind {
            Kind::Formula | Kind::Cask => {
                let brew = find_brew().ok_or_else(|| Error::Message("brew not found".into()))?;
                let flag = match pkg.kind {
                    Kind::Cask => "--cask",
                    _ => "--formula",
                };
                let status = Command::new(&brew)
                    .args(["install", flag, &pkg.name])
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::Message(format!("brew install {} failed", pkg.name)))
                }
            }
            Kind::Mas => {
                let id = pkg
                    .mas_id
                    .ok_or_else(|| Error::Message("mas entry missing id".into()))?;
                let mas = find_mas().ok_or_else(|| Error::Message("mas not found".into()))?;
                let status = Command::new(&mas)
                    .args(["install", &id.to_string()])
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::Message(format!("mas install {id} failed")))
                }
            }
        }
    }

    fn uninstall(&self, pkg: &Package) -> Result<(), Error> {
        match pkg.kind {
            Kind::Formula | Kind::Cask => {
                let brew = find_brew().ok_or_else(|| Error::Message("brew not found".into()))?;
                let flag = match pkg.kind {
                    Kind::Cask => "--cask",
                    _ => "--formula",
                };
                let status = Command::new(&brew)
                    .args(["uninstall", flag, &pkg.name])
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::Message(format!("brew uninstall {} failed", pkg.name)))
                }
            }
            Kind::Mas => {
                let id = pkg
                    .mas_id
                    .ok_or_else(|| Error::Message("mas entry missing id".into()))?;
                let mas = find_mas().ok_or_else(|| Error::Message("mas not found".into()))?;
                let status = Command::new(&mas)
                    .args(["uninstall", &id.to_string()])
                    .status()?;
                if status.success() {
                    Ok(())
                } else {
                    Err(Error::Message(format!("mas uninstall {id} failed")))
                }
            }
        }
    }

    fn brew_facts(&self, packages: &[Package]) -> Result<HashMap<PkgId, BrewFacts>, Error> {
        let wanted: Vec<&Package> = packages
            .iter()
            .filter(|p| catalog::needs_brew_facts(p))
            .collect();
        if wanted.is_empty() {
            return Ok(HashMap::new());
        }
        let Some(brew) = find_brew() else {
            return Ok(HashMap::new());
        };
        let output = Command::new(brew)
            .arg("info")
            .arg("--json=v2")
            .args(wanted.iter().map(|p| p.name.as_str()))
            .output()?;
        if !output.status.success() {
            return Ok(HashMap::new());
        }
        Ok(parse_brew_info(&String::from_utf8_lossy(&output.stdout)).unwrap_or_default())
    }
}

#[derive(serde::Deserialize)]
struct BrewInfoJson {
    #[serde(default)]
    formulae: Vec<BrewFormula>,
    #[serde(default)]
    casks: Vec<BrewCask>,
}

#[derive(serde::Deserialize)]
struct BrewFormula {
    name: String,
    desc: Option<String>,
    #[serde(default)]
    versions: BrewVersions,
    #[serde(default)]
    installed: Vec<BrewInstalled>,
}

#[derive(Default, serde::Deserialize)]
struct BrewVersions {
    stable: Option<String>,
}

#[derive(serde::Deserialize)]
struct BrewInstalled {
    version: Option<String>,
}

#[derive(serde::Deserialize)]
struct BrewCask {
    token: String,
    desc: Option<String>,
    version: Option<String>,
    installed: Option<String>,
}

fn nonempty(s: Option<String>) -> Option<String> {
    s.filter(|v| !v.is_empty())
}

fn parse_brew_info(json: &str) -> Result<HashMap<PkgId, BrewFacts>, Error> {
    let parsed: BrewInfoJson =
        serde_json::from_str(json).map_err(|e| Error::Message(format!("brew info: {e}")))?;
    let mut out = HashMap::new();
    for f in parsed.formulae {
        out.insert(
            PkgId::new(Kind::Formula, &f.name, None),
            BrewFacts {
                description: nonempty(f.desc),
                available: nonempty(f.versions.stable),
                installed: f
                    .installed
                    .into_iter()
                    .rev()
                    .find_map(|i| nonempty(i.version)),
            },
        );
    }
    for c in parsed.casks {
        out.insert(
            PkgId::new(Kind::Cask, &c.token, None),
            BrewFacts {
                description: nonempty(c.desc),
                available: nonempty(c.version),
                installed: nonempty(c.installed),
            },
        );
    }
    Ok(out)
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

fn find_mas() -> Option<PathBuf> {
    find_on_path("mas").or_else(|| {
        find_brew().and_then(|b| {
            let p = b.parent()?.join("mas");
            p.is_file().then_some(p)
        })
    })
}

fn find_brew() -> Option<PathBuf> {
    if let Some(p) = find_on_path("brew") {
        return Some(p);
    }
    for candidate in ["/opt/homebrew/bin/brew", "/usr/local/bin/brew"] {
        let p = Path::new(candidate);
        if p.is_file() {
            return Some(p.to_path_buf());
        }
    }
    None
}

fn brew_list(brew: &Path, kind: &str) -> Result<Vec<String>, Error> {
    let output = Command::new(brew)
        .args(["list", kind, "--quiet"])
        .output()?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect())
}

fn mas_list(mas: &Path) -> Result<Vec<(u64, String)>, Error> {
    let output = Command::new(mas).arg("list").output()?;
    if !output.status.success() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let mut parts = line.splitn(2, char::is_whitespace);
        let id = match parts.next().and_then(|s| s.parse().ok()) {
            Some(id) => id,
            None => continue,
        };
        let rest = parts.next().unwrap_or("").trim();
        let title = rest
            .rsplit_once(" (")
            .map(|(t, _)| t)
            .unwrap_or(rest)
            .to_string();
        rows.push((id, title));
    }
    Ok(rows)
}

const OUTCOME_LABEL_WIDTH: usize = 22;

fn pad_outcome_label(label: &str) -> String {
    format!("{label:<OUTCOME_LABEL_WIDTH$}")
}

fn stdout_styled() -> bool {
    io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

fn outcome_display(outcome: Outcome) -> (&'static str, Color, &'static str) {
    match outcome {
        Outcome::Satisfied => ("✓", Color::Green, "ready"),
        Outcome::Applied => ("+", Color::Cyan, "installed"),
        Outcome::Removed => ("−", Color::Yellow, "removed"),
        Outcome::Failed => ("✗", Color::Red, "failed"),
    }
}

pub fn print_banner() {
    let mut out = io::stdout();
    if stdout_styled() {
        let _ = execute!(
            out,
            Print("\n  "),
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Magenta),
            Print("⚡ "),
            Print(env!("CARGO_PKG_NAME")),
            ResetColor,
            Print("\n\n"),
        );
    } else {
        let _ = writeln!(out, "\n{}\n", env!("CARGO_PKG_NAME"));
    }
}

pub fn print_section(title: &str) {
    let mut out = io::stdout();
    if stdout_styled() {
        let rule = "─".repeat(title.len());
        let _ = execute!(
            out,
            Print("\n  "),
            SetAttribute(Attribute::Bold),
            SetForegroundColor(Color::Cyan),
            Print(title),
            ResetColor,
            Print("\n  "),
            SetForegroundColor(Color::DarkGrey),
            Print(rule),
            Print("\n\n"),
            ResetColor,
        );
    } else {
        let _ = writeln!(out, "\n{title}\n");
    }
}

pub fn print_goodbye() {
    let mut out = io::stdout();
    if stdout_styled() {
        let _ = execute!(
            out,
            Print("\n\n  "),
            SetForegroundColor(Color::DarkGrey),
            Print("Goodbye 👋"),
            ResetColor,
            Print("\n\n"),
        );
    } else {
        let _ = writeln!(out, "\n\nGoodbye\n\n");
    }
}

pub fn print_outcome(label: &str, outcome: Outcome) {
    let mut out = io::stdout();
    let (icon, color, text) = outcome_display(outcome);
    if stdout_styled() {
        let _ = execute!(
            out,
            Print("  "),
            SetForegroundColor(color),
            Print(icon),
            ResetColor,
            Print("  "),
            SetForegroundColor(Color::White),
            Print(pad_outcome_label(label)),
            SetForegroundColor(color),
            Print(text),
            ResetColor,
            Print("\n"),
        );
    } else {
        let _ = writeln!(out, "  {} {}", pad_outcome_label(label), outcome.label());
    }
}

#[cfg(test)]
pub struct FakeHost {
    pub clt: Cell<bool>,
    pub brew: Cell<bool>,
    pub omz: Cell<bool>,
    pub installed: RefCell<Observed>,
    pub clt_calls: Cell<u32>,
    pub brew_calls: Cell<u32>,
    pub omz_calls: Cell<u32>,
    pub installs: RefCell<Vec<PkgId>>,
    pub uninstalls: RefCell<Vec<PkgId>>,
    pub fail: Cell<bool>,
    pub facts: RefCell<HashMap<PkgId, BrewFacts>>,
}

#[cfg(test)]
impl Default for FakeHost {
    fn default() -> Self {
        Self {
            clt: Cell::new(false),
            brew: Cell::new(false),
            omz: Cell::new(false),
            installed: RefCell::new(Observed::new()),
            clt_calls: Cell::new(0),
            brew_calls: Cell::new(0),
            omz_calls: Cell::new(0),
            installs: RefCell::new(Vec::new()),
            uninstalls: RefCell::new(Vec::new()),
            fail: Cell::new(false),
            facts: RefCell::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
impl Host for FakeHost {
    fn has_clt(&self) -> bool {
        self.clt.get()
    }

    fn install_clt(&self) -> Result<(), Error> {
        self.clt_calls.set(self.clt_calls.get() + 1);
        if self.fail.get() {
            return Err(Error::Message("clt".into()));
        }
        self.clt.set(true);
        Ok(())
    }

    fn brew_bin(&self) -> Option<PathBuf> {
        self.brew
            .get()
            .then(|| PathBuf::from("/opt/homebrew/bin/brew"))
    }

    fn install_brew(&self) -> Result<(), Error> {
        self.brew_calls.set(self.brew_calls.get() + 1);
        if self.fail.get() {
            return Err(Error::Message("brew".into()));
        }
        self.brew.set(true);
        Ok(())
    }

    fn has_omz(&self) -> bool {
        self.omz.get()
    }

    fn install_omz(&self) -> Result<(), Error> {
        self.omz_calls.set(self.omz_calls.get() + 1);
        if self.fail.get() {
            return Err(Error::Message("omz".into()));
        }
        self.omz.set(true);
        Ok(())
    }

    fn installed(&self) -> Result<Observed, Error> {
        Ok(self.installed.borrow().clone())
    }

    fn install(&self, pkg: &Package) -> Result<(), Error> {
        if self.fail.get() {
            return Err(Error::Message("install".into()));
        }
        self.installs.borrow_mut().push(pkg.id.clone());
        self.installed.borrow_mut().insert(pkg.id.clone());
        Ok(())
    }

    fn uninstall(&self, pkg: &Package) -> Result<(), Error> {
        if self.fail.get() {
            return Err(Error::Message("uninstall".into()));
        }
        self.uninstalls.borrow_mut().push(pkg.id.clone());
        self.installed.borrow_mut().remove(&pkg.id);
        Ok(())
    }

    fn brew_facts(&self, packages: &[Package]) -> Result<HashMap<PkgId, BrewFacts>, Error> {
        let stored = self.facts.borrow();
        Ok(packages
            .iter()
            .filter(|p| catalog::needs_brew_facts(p))
            .filter_map(|p| stored.get(&p.id).cloned().map(|d| (p.id.clone(), d)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Kind;

    #[test]
    fn essentials_apply_then_satisfy() {
        let host = FakeHost::default();
        let first = ensure_essentials(&host);
        assert_eq!(first.clt, Outcome::Applied);
        assert_eq!(first.brew, Outcome::Applied);
        assert_eq!(first.omz, Outcome::Applied);
        assert_eq!(host.clt_calls.get(), 1);
        let second = ensure_essentials(&host);
        assert_eq!(second.clt, Outcome::Satisfied);
        assert_eq!(second.brew, Outcome::Satisfied);
        assert_eq!(second.omz, Outcome::Satisfied);
        assert_eq!(host.clt_calls.get(), 1);
        assert_eq!(host.brew_calls.get(), 1);
        assert_eq!(host.omz_calls.get(), 1);
    }

    #[test]
    fn resume_after_brew_before_omz() {
        let host = FakeHost::default();
        host.clt.set(true);
        host.brew.set(true);
        let report = ensure_essentials(&host);
        assert_eq!(report.clt, Outcome::Satisfied);
        assert_eq!(report.brew, Outcome::Satisfied);
        assert_eq!(report.omz, Outcome::Applied);
        assert_eq!(host.brew_calls.get(), 0);
        assert_eq!(host.omz_calls.get(), 1);
    }

    #[test]
    fn package_skips_when_observed() {
        let host = FakeHost::default();
        let pkg = Package {
            id: PkgId::new(Kind::Formula, "git", None),
            kind: Kind::Formula,
            name: "git".into(),
            mas_id: None,
            title: "Git".into(),
            category: "CLI".into(),
            description: None,
            available: None,
            installed_version: None,
        };
        host.installed.borrow_mut().insert(pkg.id.clone());
        assert_eq!(
            ensure_package(&host, &pkg, &host.installed.borrow()),
            Outcome::Satisfied
        );
        assert!(host.installs.borrow().is_empty());
    }

    #[test]
    fn package_applies_then_satisfies() {
        let host = FakeHost::default();
        let pkg = Package {
            id: PkgId::new(Kind::Formula, "git", None),
            kind: Kind::Formula,
            name: "git".into(),
            mas_id: None,
            title: "Git".into(),
            category: "CLI".into(),
            description: None,
            available: None,
            installed_version: None,
        };
        let empty = Observed::new();
        assert_eq!(ensure_package(&host, &pkg, &empty), Outcome::Applied);
        let observed = host.installed.borrow().clone();
        assert_eq!(ensure_package(&host, &pkg, &observed), Outcome::Satisfied);
        assert_eq!(host.installs.borrow().len(), 1);
    }

    #[test]
    fn package_removes_when_deselected() {
        let host = FakeHost::default();
        let pkg = Package {
            id: PkgId::new(Kind::Formula, "git", None),
            kind: Kind::Formula,
            name: "git".into(),
            mas_id: None,
            title: "Git".into(),
            category: "CLI".into(),
            description: None,
            available: None,
            installed_version: None,
        };
        host.installed.borrow_mut().insert(pkg.id.clone());
        let observed = host.installed.borrow().clone();
        assert_eq!(remove_package(&host, &pkg, &observed), Outcome::Removed);
        assert_eq!(*host.uninstalls.borrow(), vec![pkg.id.clone()]);
        assert!(!host.installed.borrow().contains(&pkg.id));
        assert_eq!(remove_package(&host, &pkg, &Observed::new()), Outcome::Satisfied);
    }

    #[test]
    fn parse_brew_info_maps_formula_and_cask() {
        let json = r#"{
            "formulae": [{
                "name": "git",
                "desc": "Distributed revision control system",
                "versions": {"stable": "2.55.0"},
                "installed": [{"version": "2.53.0_1"}]
            }],
            "casks": [{
                "token": "visual-studio-code",
                "desc": "Open-source code editor",
                "version": "1.135.0",
                "installed": "1.87.1"
            }]
        }"#;
        let map = parse_brew_info(json).unwrap();
        let git = &map[&PkgId::new(Kind::Formula, "git", None)];
        assert_eq!(
            git.description.as_deref(),
            Some("Distributed revision control system")
        );
        assert_eq!(git.available.as_deref(), Some("2.55.0"));
        assert_eq!(git.installed.as_deref(), Some("2.53.0_1"));
        let cask = &map[&PkgId::new(Kind::Cask, "visual-studio-code", None)];
        assert_eq!(cask.description.as_deref(), Some("Open-source code editor"));
        assert_eq!(cask.available.as_deref(), Some("1.135.0"));
        assert_eq!(cask.installed.as_deref(), Some("1.87.1"));
    }

    #[test]
    fn pad_outcome_label_fits_command_line_tools() {
        let padded = pad_outcome_label("Command Line Tools");
        assert_eq!(padded.chars().count(), OUTCOME_LABEL_WIDTH);
        assert!(padded.starts_with("Command Line Tools"));
    }

    #[test]
    fn outcome_display_maps_each_variant() {
        assert_eq!(outcome_display(Outcome::Satisfied).2, "ready");
        assert_eq!(outcome_display(Outcome::Applied).2, "installed");
        assert_eq!(outcome_display(Outcome::Removed).2, "removed");
        assert_eq!(outcome_display(Outcome::Failed).2, "failed");
    }

    #[test]
    fn cli_essentials_installs_then_satisfies() {
        let host = FakeHost::default();
        ensure_cli_essentials(&host).unwrap();
        let packages = catalog::compose(&catalog::default_loaded()).unwrap();
        for pkg in &packages {
            assert!(host.installed.borrow().contains(&pkg.id));
        }
        let installs = host.installs.borrow().len();
        ensure_cli_essentials(&host).unwrap();
        assert_eq!(host.installs.borrow().len(), installs);
    }

    #[test]
    fn fake_brew_facts_skip_mas() {
        let host = FakeHost::default();
        let git = Package {
            id: PkgId::new(Kind::Formula, "git", None),
            kind: Kind::Formula,
            name: "git".into(),
            mas_id: None,
            title: "Git".into(),
            category: "CLI".into(),
            description: Some("yaml".into()),
            available: None,
            installed_version: None,
        };
        let fact = BrewFacts {
            description: Some("brew git".into()),
            available: Some("2.55.0".into()),
            installed: Some("2.53.0_1".into()),
        };
        host.facts.borrow_mut().insert(git.id.clone(), fact.clone());
        let map = host.brew_facts(&[git]).unwrap();
        assert_eq!(map[&PkgId::new(Kind::Formula, "git", None)], fact);
    }
}

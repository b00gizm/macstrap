#[cfg(test)]
use std::cell::{Cell, RefCell};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use std::collections::HashMap;

use crate::catalog::{self, Kind, Observed, Package, PkgId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Outcome {
    Satisfied,
    Applied,
    Failed,
}

impl Outcome {
    pub fn label(self) -> &'static str {
        match self {
            Self::Satisfied => "satisfied",
            Self::Applied => "applied",
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
    fn descriptions(&self, packages: &[Package]) -> Result<HashMap<PkgId, String>, Error>;
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

pub fn ensure_package(host: &impl Host, pkg: &Package, observed: &Observed) -> Outcome {
    if observed.contains(&pkg.id) {
        return Outcome::Satisfied;
    }
    match host.install(pkg) {
        Ok(()) => Outcome::Applied,
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

    fn descriptions(&self, packages: &[Package]) -> Result<HashMap<PkgId, String>, Error> {
        let missing: Vec<&Package> = packages
            .iter()
            .filter(|p| catalog::needs_brew_desc(p))
            .collect();
        if missing.is_empty() {
            return Ok(HashMap::new());
        }
        let Some(brew) = find_brew() else {
            return Ok(HashMap::new());
        };
        let output = Command::new(brew)
            .arg("info")
            .arg("--json=v2")
            .args(missing.iter().map(|p| p.name.as_str()))
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
}

#[derive(serde::Deserialize)]
struct BrewCask {
    token: String,
    desc: Option<String>,
}

fn parse_brew_info(json: &str) -> Result<HashMap<PkgId, String>, Error> {
    let parsed: BrewInfoJson =
        serde_json::from_str(json).map_err(|e| Error::Message(format!("brew info: {e}")))?;
    let mut out = HashMap::new();
    for f in parsed.formulae {
        if let Some(desc) = f.desc.filter(|s| !s.is_empty()) {
            out.insert(PkgId::new(Kind::Formula, &f.name, None), desc);
        }
    }
    for c in parsed.casks {
        if let Some(desc) = c.desc.filter(|s| !s.is_empty()) {
            out.insert(PkgId::new(Kind::Cask, &c.token, None), desc);
        }
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

pub fn print_outcome(label: &str, outcome: Outcome) {
    let mut out = io::stdout();
    let _ = writeln!(out, "{label:<12} {}", outcome.label());
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
    pub fail: Cell<bool>,
    pub descriptions: RefCell<HashMap<PkgId, String>>,
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
            fail: Cell::new(false),
            descriptions: RefCell::new(HashMap::new()),
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

    fn descriptions(&self, packages: &[Package]) -> Result<HashMap<PkgId, String>, Error> {
        let stored = self.descriptions.borrow();
        Ok(packages
            .iter()
            .filter(|p| catalog::needs_brew_desc(p))
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
        };
        let empty = Observed::new();
        assert_eq!(ensure_package(&host, &pkg, &empty), Outcome::Applied);
        let observed = host.installed.borrow().clone();
        assert_eq!(ensure_package(&host, &pkg, &observed), Outcome::Satisfied);
        assert_eq!(host.installs.borrow().len(), 1);
    }

    #[test]
    fn parse_brew_info_maps_formula_and_cask() {
        let json = r#"{
            "formulae": [{"name": "git", "desc": "Distributed revision control system"}],
            "casks": [{"token": "visual-studio-code", "desc": "Open-source code editor"}]
        }"#;
        let map = parse_brew_info(json).unwrap();
        assert_eq!(
            map[&PkgId::new(Kind::Formula, "git", None)],
            "Distributed revision control system"
        );
        assert_eq!(
            map[&PkgId::new(Kind::Cask, "visual-studio-code", None)],
            "Open-source code editor"
        );
    }

    #[test]
    fn fake_descriptions_skip_yaml_and_mas() {
        let host = FakeHost::default();
        let git = Package {
            id: PkgId::new(Kind::Formula, "git", None),
            kind: Kind::Formula,
            name: "git".into(),
            mas_id: None,
            title: "Git".into(),
            category: "CLI".into(),
            description: Some("yaml".into()),
        };
        let jq = Package {
            id: PkgId::new(Kind::Formula, "jq", None),
            kind: Kind::Formula,
            name: "jq".into(),
            mas_id: None,
            title: "jq".into(),
            category: "CLI".into(),
            description: None,
        };
        host.descriptions
            .borrow_mut()
            .insert(jq.id.clone(), "jq desc".into());
        host.descriptions
            .borrow_mut()
            .insert(git.id.clone(), "brew git".into());
        let map = host.descriptions(&[git, jq]).unwrap();
        assert!(!map.contains_key(&PkgId::new(Kind::Formula, "git", None)));
        assert_eq!(map[&PkgId::new(Kind::Formula, "jq", None)], "jq desc");
    }
}

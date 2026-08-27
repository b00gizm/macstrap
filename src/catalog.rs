use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Formula,
    Cask,
    Mas,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Origin {
    Builtin,
    Local,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
            Self::Local => "local",
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum CatalogId {
    CliEssentials,
    NodeEssentials,
    NodeFull,
    PythonEssentials,
    PythonFull,
    RustEssentials,
    RustFull,
    Local(PathBuf),
}

enum CatalogSource {
    Embedded(&'static str),
    Path(PathBuf),
}

pub struct CatalogFile {
    pub id: CatalogId,
    pub origin: Origin,
    pub required: bool,
    source: CatalogSource,
}

impl CatalogFile {
    pub fn doc(&self) -> Result<CatalogDoc, String> {
        match &self.source {
            CatalogSource::Embedded(yaml) => load(yaml),
            CatalogSource::Path(path) => {
                let yaml = std::fs::read_to_string(path)
                    .map_err(|e| format!("{}: {e}", path.display()))?;
                load(&yaml)
            }
        }
    }

    pub fn path(&self) -> Option<&Path> {
        match &self.source {
            CatalogSource::Path(path) => Some(path),
            CatalogSource::Embedded(_) => None,
        }
    }
}

const FILES: &[CatalogFile] = &[
    CatalogFile {
        id: CatalogId::CliEssentials,
        origin: Origin::Builtin,
        required: true,
        source: CatalogSource::Embedded(include_str!("../catalogs/cli-essentials.yml")),
    },
    CatalogFile {
        id: CatalogId::NodeEssentials,
        origin: Origin::Builtin,
        required: false,
        source: CatalogSource::Embedded(include_str!("../catalogs/node-essentials.yml")),
    },
    CatalogFile {
        id: CatalogId::NodeFull,
        origin: Origin::Builtin,
        required: false,
        source: CatalogSource::Embedded(include_str!("../catalogs/node-full.yml")),
    },
    CatalogFile {
        id: CatalogId::PythonEssentials,
        origin: Origin::Builtin,
        required: false,
        source: CatalogSource::Embedded(include_str!("../catalogs/python-essentials.yml")),
    },
    CatalogFile {
        id: CatalogId::PythonFull,
        origin: Origin::Builtin,
        required: false,
        source: CatalogSource::Embedded(include_str!("../catalogs/python-full.yml")),
    },
    CatalogFile {
        id: CatalogId::RustEssentials,
        origin: Origin::Builtin,
        required: false,
        source: CatalogSource::Embedded(include_str!("../catalogs/rust-essentials.yml")),
    },
    CatalogFile {
        id: CatalogId::RustFull,
        origin: Origin::Builtin,
        required: false,
        source: CatalogSource::Embedded(include_str!("../catalogs/rust-full.yml")),
    },
];

pub fn default_config_dir() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".config").join(env!("CARGO_PKG_NAME"))
}

pub fn discover_local(config_dir: &Path) -> Result<Vec<CatalogFile>, String> {
    let mut out = Vec::new();
    let read_dir = match std::fs::read_dir(config_dir) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(format!("{}: {e}", config_dir.display())),
    };
    for entry in read_dir {
        let entry = entry.map_err(|e| format!("{}: {e}", config_dir.display()))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        if ext != "yml" && ext != "yaml" {
            continue;
        }
        if path
            .file_name()
            .is_some_and(|n| n == "config.yml" || n == "config.yaml")
        {
            continue;
        }
        let path = path.canonicalize().unwrap_or(path);
        let id = CatalogId::Local(path.clone());
        out.push(CatalogFile {
            id,
            origin: Origin::Local,
            required: false,
            source: CatalogSource::Path(path),
        });
    }
    out.sort_by(|a, b| {
        a.path()
            .and_then(|p| p.file_name())
            .cmp(&b.path().and_then(|p| p.file_name()))
    });
    Ok(out)
}

pub fn all_entries(config_dir: &Path) -> Result<Vec<CatalogEntry>, String> {
    let mut all: Vec<CatalogEntry> = FILES.iter().map(|f| CatalogEntry::Builtin(f)).collect();
    for local in discover_local(config_dir)? {
        all.push(CatalogEntry::Owned(local));
    }
    Ok(all)
}

pub enum CatalogEntry {
    Builtin(&'static CatalogFile),
    Owned(CatalogFile),
}

impl CatalogEntry {
    pub fn file(&self) -> &CatalogFile {
        match self {
            Self::Builtin(f) => f,
            Self::Owned(f) => f,
        }
    }
}

pub fn infer_title_from_filename(filename: &str) -> String {
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    stem.split(['-', '_'])
        .filter(|w| !w.is_empty())
        .map(title_case_word)
        .collect::<Vec<_>>()
        .join(" ")
}

fn title_case_word(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

pub struct CreateCatalogInput {
    pub filename: String,
    pub title: String,
    pub description: Option<String>,
    pub location: PathBuf,
}

pub fn create_catalog(input: CreateCatalogInput) -> Result<PathBuf, String> {
    let filename = input.filename.trim();
    if filename.is_empty() {
        return Err("file name is required".into());
    }
    if filename.contains('/') || filename.contains('\\') {
        return Err("file name must not contain path separators".into());
    }
    let path = input.location.join(filename);
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    std::fs::create_dir_all(&input.location)
        .map_err(|e| format!("{}: {e}", input.location.display()))?;
    let doc = CatalogDoc {
        title: input.title,
        description: input.description.filter(|d| !d.is_empty()),
        packages: Vec::new(),
    };
    write_catalog_doc(&path, &doc)?;
    Ok(path.canonicalize().unwrap_or(path))
}

pub fn default_loaded() -> HashSet<CatalogId> {
    FILES
        .iter()
        .filter(|f| f.required)
        .map(|f| f.id.clone())
        .collect()
}

pub fn is_required_catalog(id: &CatalogId) -> bool {
    FILES
        .iter()
        .find(|f| &f.id == id)
        .is_some_and(|f| f.required)
}

fn catalog_persist_key(id: &CatalogId) -> Option<String> {
    match id {
        CatalogId::CliEssentials => None,
        CatalogId::NodeEssentials => Some("node-essentials".into()),
        CatalogId::NodeFull => Some("node-full".into()),
        CatalogId::PythonEssentials => Some("python-essentials".into()),
        CatalogId::PythonFull => Some("python-full".into()),
        CatalogId::RustEssentials => Some("rust-essentials".into()),
        CatalogId::RustFull => Some("rust-full".into()),
        CatalogId::Local(path) => {
            let path = path.canonicalize().unwrap_or_else(|_| path.clone());
            Some(path.display().to_string())
        }
    }
}

fn parse_persisted_catalog(key: &str, config_dir: &Path) -> Option<CatalogId> {
    match key {
        "node-essentials" => Some(CatalogId::NodeEssentials),
        "node-full" => Some(CatalogId::NodeFull),
        "python-essentials" => Some(CatalogId::PythonEssentials),
        "python-full" => Some(CatalogId::PythonFull),
        "rust-essentials" => Some(CatalogId::RustEssentials),
        "rust-full" => Some(CatalogId::RustFull),
        _ => {
            let path = PathBuf::from(key);
            let path = if path.is_file() {
                path
            } else {
                config_dir.join(key)
            };
            if path.is_file() {
                Some(CatalogId::Local(path.canonicalize().unwrap_or(path)))
            } else {
                None
            }
        }
    }
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct PersistedConfig {
    #[serde(default)]
    catalogs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    desired: Option<Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PersistIntent {
    Absent,
    Recorded(Desired),
}

impl PersistIntent {
    pub fn desired(&self) -> Desired {
        match self {
            Self::Absent => Desired::new(),
            Self::Recorded(desired) => desired.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Persisted {
    pub catalogs: HashSet<CatalogId>,
    pub intent: PersistIntent,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Append {
    Added,
    AlreadyPresent,
}

pub fn load_persisted(config_dir: &Path) -> Persisted {
    let mut catalogs = default_loaded();
    let path = config_dir.join("config.yml");
    let Ok(yaml) = std::fs::read_to_string(&path) else {
        return Persisted {
            catalogs,
            intent: PersistIntent::Absent,
        };
    };
    let Ok(config) = serde_yaml::from_str::<PersistedConfig>(&yaml) else {
        return Persisted {
            catalogs,
            intent: PersistIntent::Absent,
        };
    };
    for key in config.catalogs {
        if let Some(id) = parse_persisted_catalog(&key, config_dir) {
            if !is_required_catalog(&id) {
                catalogs.insert(id);
            }
        }
    }
    let intent = match config.desired {
        None => PersistIntent::Absent,
        Some(ids) => PersistIntent::Recorded(
            ids.into_iter()
                .filter_map(|raw| PkgId::parse(&raw))
                .filter(|id| !is_protected(id))
                .map(|id| {
                    let package = stub_from_id(&id);
                    (id, package)
                })
                .collect(),
        ),
    };
    Persisted { catalogs, intent }
}

pub fn save_persisted(
    config_dir: &Path,
    catalogs: &HashSet<CatalogId>,
    intent: &PersistIntent,
) -> Result<(), String> {
    std::fs::create_dir_all(config_dir).map_err(|e| format!("{}: {e}", config_dir.display()))?;
    let mut catalogs: Vec<String> = catalogs.iter().filter_map(catalog_persist_key).collect();
    catalogs.sort();
    let desired = match intent {
        PersistIntent::Absent => None,
        PersistIntent::Recorded(desired) => {
            let mut ids: Vec<String> = desired
                .keys()
                .filter(|id| !is_protected(id))
                .map(|id| id.as_str().to_string())
                .collect();
            ids.sort();
            Some(ids)
        }
    };
    write_yaml_atomically(
        &config_dir.join("config.yml"),
        &PersistedConfig { catalogs, desired },
    )
}

static PROTECTED: LazyLock<HashSet<PkgId>> = LazyLock::new(|| {
    FILES
        .iter()
        .filter(|f| f.required)
        .flat_map(|f| f.doc().expect("embedded catalogs parse").packages)
        .map(|pkg| pkg.id)
        .collect()
});

pub fn is_protected(id: &PkgId) -> bool {
    PROTECTED.contains(id)
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PkgId(String);

impl PkgId {
    pub fn new(kind: Kind, name: &str, mas_id: Option<u64>) -> Self {
        match (kind, mas_id) {
            (Kind::Mas, Some(id)) => Self(format!("mas:{id}")),
            (Kind::Mas, None) => Self(format!("mas:{name}")),
            (Kind::Formula, _) => Self(format!("formula:{name}")),
            (Kind::Cask, _) => Self(format!("cask:{name}")),
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn parse(raw: &str) -> Option<Self> {
        if let Some(name) = raw.strip_prefix("formula:").filter(|name| !name.is_empty()) {
            return Some(Self::new(Kind::Formula, name, None));
        }
        if let Some(name) = raw.strip_prefix("cask:").filter(|name| !name.is_empty()) {
            return Some(Self::new(Kind::Cask, name, None));
        }
        let id = raw.strip_prefix("mas:")?.parse().ok()?;
        Some(Self::new(Kind::Mas, raw, Some(id)))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Package {
    pub id: PkgId,
    pub kind: Kind,
    pub name: String,
    pub mas_id: Option<u64>,
    pub title: String,
    pub category: String,
    pub description: Option<String>,
    pub available: Option<String>,
    pub installed_version: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BrewFacts {
    pub description: Option<String>,
    pub available: Option<String>,
    pub installed: Option<String>,
}

pub type Catalog = Vec<Package>;
pub type Desired = HashMap<PkgId, Package>;
pub type Observed = HashSet<PkgId>;
pub type Selection = HashMap<PkgId, bool>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogDoc {
    pub title: String,
    pub description: Option<String>,
    pub packages: Catalog,
}

#[derive(Debug, Deserialize, Serialize)]
struct CatalogYaml {
    title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    packages: Vec<CatalogRow>,
}

#[derive(Debug, Deserialize, Serialize)]
struct CatalogRow {
    kind: Kind,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    mas_id: Option<u64>,
    title: String,
    category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}

impl From<&Package> for CatalogRow {
    fn from(package: &Package) -> Self {
        Self {
            kind: package.kind,
            name: package.name.clone(),
            mas_id: package.mas_id,
            title: package.title.clone(),
            category: package.category.clone(),
            description: package.description.clone(),
        }
    }
}

impl From<CatalogRow> for Package {
    fn from(row: CatalogRow) -> Self {
        let id = PkgId::new(row.kind, &row.name, row.mas_id);
        Self {
            id,
            kind: row.kind,
            name: row.name,
            mas_id: row.mas_id,
            title: row.title,
            category: row.category,
            description: row.description,
            available: None,
            installed_version: None,
        }
    }
}

pub fn load(yaml: &str) -> Result<CatalogDoc, String> {
    let doc: CatalogYaml = serde_yaml::from_str(yaml).map_err(|e| format!("catalog: {e}"))?;
    let mut packages = Vec::with_capacity(doc.packages.len());
    for row in doc.packages {
        if row.kind == Kind::Mas && row.mas_id.is_none() {
            return Err(format!(
                "catalog: mas package {} requires mas_id",
                row.title
            ));
        }
        packages.push(Package::from(row));
    }
    Ok(CatalogDoc {
        title: doc.title,
        description: doc.description,
        packages,
    })
}

pub fn compose(loaded: &HashSet<CatalogId>) -> Result<Catalog, String> {
    let config_dir = default_config_dir();
    let mut active = default_loaded();
    active.extend(loaded.iter().cloned());
    let mut packages = Catalog::new();
    let mut seen = HashSet::new();
    for entry in all_entries(&config_dir)? {
        let file = entry.file();
        if !active.contains(&file.id) {
            continue;
        }
        for pkg in file.doc()?.packages {
            if seen.insert(pkg.id.clone()) {
                packages.push(pkg);
            }
        }
    }
    Ok(packages)
}

pub fn compose_all() -> Result<Catalog, String> {
    let config_dir = default_config_dir();
    let all: HashSet<CatalogId> = all_entries(&config_dir)?
        .into_iter()
        .map(|e| e.file().id.clone())
        .collect();
    compose(&all)
}

pub struct Merge {
    pub packages: Catalog,
    pub selection: Selection,
}

pub fn merge(catalog: &[Package], desired: &Desired) -> Merge {
    let mut packages = catalog.to_vec();
    let mut selection = Selection::new();
    for pkg in &packages {
        selection.insert(
            pkg.id.clone(),
            desired.contains_key(&pkg.id) || is_protected(&pkg.id),
        );
    }
    for (id, pkg) in desired {
        if !packages.iter().any(|p| &p.id == id) {
            packages.push(pkg.clone());
            selection.insert(id.clone(), true);
        }
    }
    packages.sort_by(|a, b| {
        a.title
            .to_ascii_lowercase()
            .cmp(&b.title.to_ascii_lowercase())
    });
    Merge {
        packages,
        selection,
    }
}

pub fn checked_desired(packages: &[Package], selection: &Selection) -> Desired {
    packages
        .iter()
        .filter(|pkg| selection.get(&pkg.id).copied().unwrap_or(false) && !is_protected(&pkg.id))
        .map(|pkg| (pkg.id.clone(), pkg.clone()))
        .collect()
}

fn union_desired(mut persist: Desired, brewfile: &Desired) -> Desired {
    persist.extend(brewfile.clone());
    persist
}

pub fn assemble(
    catalog: &[Package],
    brewfile: &Desired,
    persist: &PersistIntent,
    observed: &Observed,
    universe: &[Package],
    show_all_installed: bool,
) -> Merge {
    let desired = union_desired(persist.desired(), brewfile);
    let mut merged = merge(catalog, &desired);
    include_observed(&mut merged, observed, universe, show_all_installed);
    if matches!(persist, PersistIntent::Absent) {
        preselect_installed(&mut merged.selection, observed, None);
    }
    merged
}

pub fn local_files(entries: &[CatalogEntry]) -> Vec<&CatalogFile> {
    let mut files: Vec<&CatalogFile> = entries
        .iter()
        .map(CatalogEntry::file)
        .filter(|file| file.origin == Origin::Local)
        .collect();
    files.sort_by(|a, b| {
        a.path()
            .and_then(Path::file_name)
            .cmp(&b.path().and_then(Path::file_name))
    });
    files
}

pub fn append_local_package(path: &Path, package: &Package) -> Result<Append, String> {
    if !path.is_file() {
        return Err(format!("{} is not an existing catalog", path.display()));
    }
    if path
        .file_name()
        .is_some_and(|name| name == "config.yml" || name == "config.yaml")
    {
        return Err("config.yml is not a catalog".into());
    }
    if !matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some("yml" | "yaml")
    ) {
        return Err(format!("{} is not a YAML catalog", path.display()));
    }
    if package.kind == Kind::Mas && package.mas_id.is_none() {
        return Err(format!(
            "catalog: mas package {} requires mas_id",
            package.title
        ));
    }
    let yaml =
        std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    let mut doc = load(&yaml)?;
    if doc
        .packages
        .iter()
        .any(|existing| existing.id == package.id)
    {
        return Ok(Append::AlreadyPresent);
    }
    doc.packages.push(package.clone());
    write_catalog_doc(path, &doc)?;
    Ok(Append::Added)
}

fn write_catalog_doc(path: &Path, doc: &CatalogDoc) -> Result<(), String> {
    let yaml = CatalogYaml {
        title: doc.title.clone(),
        description: doc.description.clone(),
        packages: doc.packages.iter().map(CatalogRow::from).collect(),
    };
    write_yaml_atomically(path, &yaml)
}

fn write_yaml_atomically<T: Serialize>(path: &Path, document: &T) -> Result<(), String> {
    let yaml = serde_yaml::to_string(document).map_err(|error| format!("yaml: {error}"))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("macstrap");
    let mut attempt = 0;
    let (temporary, mut file) = loop {
        let temporary = parent.join(format!(
            ".{filename}.{}.{}.tmp",
            std::process::id(),
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => attempt += 1,
            Err(error) => return Err(format!("{}: {error}", temporary.display())),
        }
    };
    let result = (|| {
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())
                .map_err(|error| format!("{}: {error}", temporary.display()))?;
        }
        file.write_all(yaml.as_bytes())
            .map_err(|error| format!("{}: {error}", temporary.display()))?;
        file.sync_all()
            .map_err(|error| format!("{}: {error}", temporary.display()))?;
        std::fs::rename(&temporary, path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| format!("{}: {error}", parent.display()))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
    result
}

pub fn include_observed(
    merged: &mut Merge,
    observed: &Observed,
    universe: &[Package],
    show_all_installed: bool,
) {
    for id in observed {
        if merged.packages.iter().any(|p| &p.id == id) {
            continue;
        }
        let pkg = match universe.iter().find(|p| &p.id == id) {
            Some(pkg) => pkg.clone(),
            None if show_all_installed => stub_from_id(id),
            None => continue,
        };
        merged.selection.insert(id.clone(), false);
        merged.packages.push(pkg);
    }
    merged.packages.sort_by(|a, b| {
        a.title
            .to_ascii_lowercase()
            .cmp(&b.title.to_ascii_lowercase())
    });
}

fn stub_from_id(id: &PkgId) -> Package {
    let (kind, name, mas_id) = parse_id(id);
    Package {
        id: id.clone(),
        kind,
        name: name.clone(),
        mas_id,
        title: name,
        category: "Installed".into(),
        description: None,
        available: None,
        installed_version: None,
    }
}

fn parse_id(id: &PkgId) -> (Kind, String, Option<u64>) {
    let s = &id.0;
    if let Some(name) = s.strip_prefix("formula:") {
        return (Kind::Formula, name.to_string(), None);
    }
    if let Some(name) = s.strip_prefix("cask:") {
        return (Kind::Cask, name.to_string(), None);
    }
    if let Some(rest) = s.strip_prefix("mas:") {
        if let Ok(n) = rest.parse() {
            return (Kind::Mas, rest.to_string(), Some(n));
        }
        return (Kind::Mas, rest.to_string(), None);
    }
    (Kind::Formula, s.clone(), None)
}

pub fn needs_brew_facts(pkg: &Package) -> bool {
    matches!(pkg.kind, Kind::Formula | Kind::Cask)
}

pub fn apply_facts(packages: &mut [Package], facts: &HashMap<PkgId, BrewFacts>) {
    for pkg in packages {
        if !needs_brew_facts(pkg) {
            continue;
        }
        let Some(fact) = facts.get(&pkg.id) else {
            continue;
        };
        if pkg.description.is_none() {
            pkg.description = fact.description.clone();
        }
        pkg.available = fact.available.clone();
        pkg.installed_version = fact.installed.clone();
    }
}

pub fn preselect_installed(
    selection: &mut Selection,
    observed: &Observed,
    prior: Option<&Selection>,
) {
    for id in observed {
        if !selection.contains_key(id) {
            continue;
        }
        if prior.is_some_and(|p| p.contains_key(id)) {
            continue;
        }
        selection.insert(id.clone(), true);
    }
}

pub fn pending<'a>(
    packages: &'a [Package],
    selection: &Selection,
    observed: &Observed,
) -> Vec<&'a Package> {
    packages
        .iter()
        .filter(|pkg| {
            selection.get(&pkg.id).copied().unwrap_or(false) && !observed.contains(&pkg.id)
        })
        .collect()
}

pub fn pending_uninstall<'a>(
    packages: &'a [Package],
    selection: &Selection,
    observed: &Observed,
) -> Vec<&'a Package> {
    packages
        .iter()
        .filter(|pkg| {
            !is_protected(&pkg.id)
                && !selection.get(&pkg.id).copied().unwrap_or(false)
                && observed.contains(&pkg.id)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::brewfile;

    fn pkg(kind: Kind, name: &str, mas_id: Option<u64>) -> Package {
        let id = PkgId::new(kind, name, mas_id);
        Package {
            id,
            kind,
            name: name.to_string(),
            mas_id,
            title: name.to_string(),
            category: "Test".to_string(),
            description: None,
            available: None,
            installed_version: None,
        }
    }

    fn names(catalog: &[Package]) -> HashSet<&str> {
        catalog.iter().map(|p| p.name.as_str()).collect()
    }

    #[test]
    fn compose_always_includes_cli_essentials() {
        let catalog = compose(&HashSet::new()).unwrap();
        let names = names(&catalog);
        assert!(names.contains("git"));
        assert!(names.contains("jq"));
        assert!(!names.contains("node"));
        assert!(!names.contains("rustup"));
        assert!(!names.contains("python@3.14"));
    }

    #[test]
    fn compose_unions_and_dedups() {
        let loaded = HashSet::from([CatalogId::NodeEssentials, CatalogId::NodeFull]);
        let catalog = compose(&loaded).unwrap();
        let names = names(&catalog);
        assert!(names.contains("git"));
        assert!(names.contains("node"));
        assert!(names.contains("pnpm"));
        assert!(names.contains("bun"));
        assert_eq!(
            catalog.iter().filter(|p| p.name == "node").count(),
            1,
            "node from essentials and full must collapse to one row"
        );
    }

    #[test]
    fn yaml_mas_parses_with_mas_id() {
        let doc = load(
            r#"
title: Mac App Store essentials
packages:
  - name: Yoink
    kind: mas
    mas_id: 457622435
    title: Yoink
    category: Utilities
"#,
        )
        .unwrap();
        let pkg = &doc.packages[0];
        assert_eq!(pkg.kind, Kind::Mas);
        assert_eq!(pkg.mas_id, Some(457622435));
        assert_eq!(pkg.id, PkgId::new(Kind::Mas, "Yoink", Some(457622435)));
    }

    #[test]
    fn yaml_mas_requires_mas_id() {
        let err = load(
            r#"
title: Bad
packages:
  - name: Yoink
    kind: mas
    title: Yoink
    category: Utilities
"#,
        )
        .unwrap_err();
        assert!(err.contains("mas_id"), "{err}");
    }

    #[test]
    fn mas_catalog_merges_into_picker() {
        let doc = load(
            r#"
title: Mac App Store essentials
packages:
  - name: Fantastical
    kind: mas
    mas_id: 975937182
    title: Fantastical
    category: Productivity
"#,
        )
        .unwrap();
        let merged = merge(&doc.packages, &Desired::new());
        let id = PkgId::new(Kind::Mas, "Fantastical", Some(975937182));
        assert!(merged.packages.iter().any(|p| p.id == id));
        assert!(!merged.selection[&id]);
    }

    #[test]
    fn yaml_description_is_optional() {
        let doc = load(
            r#"
title: Example
packages:
  - name: git
    kind: formula
    title: Git
    category: CLI
    description: Distributed version control
  - name: jq
    kind: formula
    title: jq
    category: CLI
"#,
        )
        .unwrap();
        assert_eq!(doc.title, "Example");
        assert_eq!(doc.description, None);
        assert_eq!(
            doc.packages[0].description.as_deref(),
            Some("Distributed version control")
        );
        assert_eq!(doc.packages[1].description, None);
    }

    #[test]
    fn apply_facts_fills_gaps_keeps_yaml_and_sets_versions() {
        let mut packages = vec![
            pkg(Kind::Formula, "git", None),
            pkg(Kind::Formula, "jq", None),
            pkg(Kind::Mas, "Yoink", Some(457622435)),
        ];
        packages[0].description = Some("from yaml".into());
        let mut facts = HashMap::new();
        facts.insert(
            PkgId::new(Kind::Formula, "git", None),
            BrewFacts {
                description: Some("from brew".into()),
                available: Some("2.55.0".into()),
                installed: Some("2.53.0_1".into()),
            },
        );
        facts.insert(
            PkgId::new(Kind::Formula, "jq", None),
            BrewFacts {
                description: Some("jq desc".into()),
                available: Some("1.8.2".into()),
                installed: None,
            },
        );
        apply_facts(&mut packages, &facts);
        assert_eq!(packages[0].description.as_deref(), Some("from yaml"));
        assert_eq!(packages[0].available.as_deref(), Some("2.55.0"));
        assert_eq!(packages[0].installed_version.as_deref(), Some("2.53.0_1"));
        assert_eq!(packages[1].description.as_deref(), Some("jq desc"));
        assert_eq!(packages[1].available.as_deref(), Some("1.8.2"));
        assert_eq!(packages[1].installed_version, None);
        assert_eq!(packages[2].description, None);
        assert_eq!(packages[2].available, None);
        assert!(needs_brew_facts(&pkg(Kind::Formula, "fd", None)));
        assert!(!needs_brew_facts(&packages[2]));
    }

    #[test]
    fn bundled_files_are_builtin() {
        assert!(
            FILES
                .iter()
                .all(|f| f.origin == Origin::Builtin && f.origin.label() == "builtin")
        );
    }

    #[test]
    fn each_catalog_file_parses() {
        for file in FILES {
            let doc = file.doc().unwrap();
            assert!(!doc.title.is_empty());
            assert!(
                !doc.packages.is_empty(),
                "{} must list at least one package",
                doc.title
            );
        }
    }

    #[test]
    fn merge_sorts_by_title() {
        let catalog = vec![
            pkg(Kind::Formula, "jq", None),
            pkg(Kind::Formula, "Git", None),
            pkg(Kind::Formula, "fd", None),
        ];
        let merged = merge(&catalog, &Desired::new());
        let titles: Vec<&str> = merged.packages.iter().map(|p| p.title.as_str()).collect();
        assert_eq!(titles, ["fd", "Git", "jq"]);
    }

    #[test]
    fn merge_checks_desired_and_appends_extras() {
        let catalog = vec![
            pkg(Kind::Formula, "git", None),
            pkg(Kind::Formula, "ripgrep", None),
            pkg(Kind::Cask, "visual-studio-code", None),
        ];
        let parsed = brewfile::parse(include_str!("../testdata/Brewfile"));
        let merged = merge(&catalog, &parsed.desired);

        assert!(merged.selection[&PkgId::new(Kind::Formula, "git", None)]);
        assert!(!merged.selection[&PkgId::new(Kind::Formula, "ripgrep", None)]);
        assert!(merged.selection[&PkgId::new(Kind::Cask, "visual-studio-code", None)]);
        assert!(
            merged
                .packages
                .iter()
                .any(|p| p.id == PkgId::new(Kind::Formula, "coreutils", None))
        );
        assert!(
            merged
                .packages
                .iter()
                .any(|p| p.id == PkgId::new(Kind::Mas, "Fantastical", Some(975937182)))
        );
        assert!(merged.selection[&PkgId::new(Kind::Mas, "Fantastical", Some(975937182))]);
    }

    #[test]
    fn pending_skips_observed() {
        let git = pkg(Kind::Formula, "git", None);
        let rg = pkg(Kind::Formula, "ripgrep", None);
        let mut selection = Selection::new();
        selection.insert(git.id.clone(), true);
        selection.insert(rg.id.clone(), true);
        let mut observed = Observed::new();
        observed.insert(git.id.clone());
        let pkgs = [git, rg];
        let out = pending(&pkgs, &selection, &observed);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "ripgrep");
    }

    #[test]
    fn preselect_installed_checks_observed_rows() {
        let git = pkg(Kind::Formula, "git", None);
        let rg = pkg(Kind::Formula, "ripgrep", None);
        let mut selection = Selection::new();
        selection.insert(git.id.clone(), false);
        selection.insert(rg.id.clone(), false);
        let mut observed = Observed::new();
        observed.insert(git.id.clone());
        preselect_installed(&mut selection, &observed, None);
        assert!(selection[&git.id]);
        assert!(!selection[&rg.id]);
    }

    #[test]
    fn pending_uninstall_deselected_observed() {
        let git = pkg(Kind::Formula, "git", None);
        let rg = pkg(Kind::Formula, "ripgrep", None);
        let mut selection = Selection::new();
        selection.insert(git.id.clone(), true);
        selection.insert(rg.id.clone(), false);
        let mut observed = Observed::new();
        observed.insert(git.id.clone());
        observed.insert(rg.id.clone());
        let pkgs = [git, rg];
        let out = pending_uninstall(&pkgs, &selection, &observed);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "ripgrep");
    }

    #[test]
    fn include_observed_adds_installed_from_unloaded_catalog() {
        let catalog = compose(&HashSet::new()).unwrap();
        let mut merged = merge(&catalog, &Desired::new());
        let node_id = PkgId::new(Kind::Formula, "node", None);
        let mut observed = Observed::new();
        observed.insert(node_id.clone());
        let universe = compose_all().unwrap();
        include_observed(&mut merged, &observed, &universe, false);
        assert!(merged.packages.iter().any(|p| p.name == "node"));
        assert!(merged.selection.contains_key(&node_id));
        assert!(!merged.selection[&node_id]);
    }

    #[test]
    fn include_observed_skips_rows_already_present() {
        let loaded = HashSet::from([CatalogId::NodeEssentials]);
        let catalog = compose(&loaded).unwrap();
        let mut merged = merge(&catalog, &Desired::new());
        let node_id = PkgId::new(Kind::Formula, "node", None);
        let mut observed = Observed::new();
        observed.insert(node_id.clone());
        let before = merged.packages.len();
        include_observed(&mut merged, &observed, &catalog, false);
        assert_eq!(merged.packages.len(), before);
    }

    #[test]
    fn include_observed_stubs_unknown_formula() {
        let catalog = compose(&HashSet::new()).unwrap();
        let mut merged = merge(&catalog, &Desired::new());
        let id = PkgId::new(Kind::Formula, "wget", None);
        let mut observed = Observed::new();
        observed.insert(id.clone());
        include_observed(&mut merged, &observed, &catalog, true);
        let wget = merged.packages.iter().find(|p| p.name == "wget").unwrap();
        assert_eq!(wget.category, "Installed");
        assert_eq!(wget.title, "wget");
    }

    #[test]
    fn include_observed_catalog_mode_skips_unknown_formula() {
        let catalog = compose(&HashSet::new()).unwrap();
        let mut merged = merge(&catalog, &Desired::new());
        let id = PkgId::new(Kind::Formula, "wget", None);
        let mut observed = Observed::new();
        observed.insert(id.clone());
        include_observed(&mut merged, &observed, &catalog, false);
        assert!(!merged.packages.iter().any(|p| p.name == "wget"));
    }

    #[test]
    fn infer_title_from_filename_splits_on_dashes() {
        assert_eq!(
            infer_title_from_filename("foo-essentials.yaml"),
            "Foo Essentials"
        );
        assert_eq!(infer_title_from_filename("my_custom.yml"), "My Custom");
    }

    #[test]
    fn create_catalog_writes_empty_packages() {
        let dir = std::env::temp_dir().join(format!("macstrap-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = create_catalog(CreateCatalogInput {
            filename: "foo-essentials.yaml".into(),
            title: "Foo Essentials".into(),
            description: Some("My tools".into()),
            location: dir.clone(),
        })
        .unwrap();
        assert_eq!(path.file_name(), Some("foo-essentials.yaml".as_ref()));
        let doc = load(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(doc.title, "Foo Essentials");
        assert_eq!(doc.description.as_deref(), Some("My tools"));
        assert!(doc.packages.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_state_roundtrips_catalogs_and_desired() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("macstrap-persist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = create_catalog(CreateCatalogInput {
            filename: "mine.yml".into(),
            title: "Mine".into(),
            description: None,
            location: dir.clone(),
        })
        .unwrap();
        let mut loaded = default_loaded();
        loaded.insert(CatalogId::NodeEssentials);
        loaded.insert(CatalogId::RustFull);
        loaded.insert(CatalogId::Local(path.clone()));
        let desired = Desired::from([(
            PkgId::new(Kind::Formula, "ripgrep", None),
            pkg(Kind::Formula, "ripgrep", None),
        )]);
        save_persisted(&dir, &loaded, &PersistIntent::Recorded(desired)).unwrap();
        let config_path = dir.join("config.yml");
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let restored = load_persisted(&dir);
        save_persisted(&dir, &restored.catalogs, &restored.intent).unwrap();
        assert_eq!(
            std::fs::metadata(&config_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let yaml = std::fs::read_to_string(&config_path).unwrap();
        assert!(yaml.contains("catalogs:"));
        assert!(yaml.contains("node-essentials"));
        assert!(yaml.contains("rust-full"));
        assert!(!yaml.contains("cli-essentials"));
        assert!(yaml.contains(&path.display().to_string()));
        assert!(yaml.contains("formula:ripgrep"));
        let restored = load_persisted(&dir);
        assert!(restored.catalogs.contains(&CatalogId::CliEssentials));
        assert!(restored.catalogs.contains(&CatalogId::NodeEssentials));
        assert!(restored.catalogs.contains(&CatalogId::RustFull));
        assert!(restored.catalogs.contains(&CatalogId::Local(path)));
        let PersistIntent::Recorded(desired) = restored.intent else {
            panic!("desired must be recorded");
        };
        assert!(desired.contains_key(&PkgId::new(Kind::Formula, "ripgrep", None)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_yml_is_not_a_catalog() {
        let dir = std::env::temp_dir().join(format!("macstrap-config-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut loaded = default_loaded();
        loaded.insert(CatalogId::PythonEssentials);
        save_persisted(&dir, &loaded, &PersistIntent::Absent).unwrap();
        let entries = all_entries(&dir).unwrap();
        assert!(
            entries
                .iter()
                .all(|e| e.file().path() != Some(dir.join("config.yml").as_path()))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_missing_and_empty_desired_have_different_intents() {
        let dir = std::env::temp_dir().join(format!("macstrap-intent-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(matches!(load_persisted(&dir).intent, PersistIntent::Absent));
        std::fs::write(dir.join("config.yml"), "catalogs: []\n").unwrap();
        assert!(matches!(load_persisted(&dir).intent, PersistIntent::Absent));
        std::fs::write(dir.join("config.yml"), "catalogs: []\ndesired: []\n").unwrap();
        assert!(matches!(
            load_persisted(&dir).intent,
            PersistIntent::Recorded(desired) if desired.is_empty()
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persisted_ids_are_valid_sorted_and_exclude_protected() {
        let dir = std::env::temp_dir().join(format!("macstrap-ids-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = pkg(Kind::Formula, "git", None);
        let rg = pkg(Kind::Formula, "ripgrep", None);
        let code = pkg(Kind::Cask, "visual-studio-code", None);
        let desired = Desired::from([
            (git.id.clone(), git),
            (rg.id.clone(), rg),
            (code.id.clone(), code),
        ]);
        save_persisted(&dir, &default_loaded(), &PersistIntent::Recorded(desired)).unwrap();
        let yaml = std::fs::read_to_string(dir.join("config.yml")).unwrap();
        assert!(!yaml.contains("formula:git"));
        assert!(yaml.find("cask:visual-studio-code") < yaml.find("formula:ripgrep"));
        std::fs::write(
            dir.join("config.yml"),
            "catalogs: []\ndesired:\n  - nope\n  - mas:not-a-number\n  - formula:ripgrep\n",
        )
        .unwrap();
        let PersistIntent::Recorded(desired) = load_persisted(&dir).intent else {
            panic!("desired must be recorded");
        };
        assert_eq!(desired.len(), 1);
        assert!(desired.contains_key(&PkgId::new(Kind::Formula, "ripgrep", None)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_preselects_only_when_persist_is_absent() {
        let rg = pkg(Kind::Formula, "ripgrep", None);
        let observed = Observed::from([rg.id.clone()]);
        let absent = assemble(
            std::slice::from_ref(&rg),
            &Desired::new(),
            &PersistIntent::Absent,
            &observed,
            &[],
            false,
        );
        assert!(absent.selection[&rg.id]);
        let recorded = assemble(
            std::slice::from_ref(&rg),
            &Desired::new(),
            &PersistIntent::Recorded(Desired::new()),
            &observed,
            &[],
            false,
        );
        assert!(!recorded.selection[&rg.id]);
    }

    #[test]
    fn checked_desired_omits_protected_and_merge_restores_them() {
        let git = pkg(Kind::Formula, "git", None);
        let rg = pkg(Kind::Formula, "ripgrep", None);
        let packages = vec![git.clone(), rg.clone()];
        let selection = Selection::from([(git.id.clone(), true), (rg.id.clone(), true)]);
        let desired = checked_desired(&packages, &selection);
        assert!(!desired.contains_key(&git.id));
        assert!(desired.contains_key(&rg.id));
        let merged = merge(&packages, &Desired::new());
        assert!(merged.selection[&git.id]);
        assert!(!merged.selection[&rg.id]);
    }

    #[test]
    fn append_local_package_is_idempotent_and_preserves_permissions() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let dir = std::env::temp_dir().join(format!("macstrap-append-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = create_catalog(CreateCatalogInput {
            filename: "mine.yml".into(),
            title: "Mine".into(),
            description: Some("Personal tools".into()),
            location: dir.clone(),
        })
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
        let package = pkg(Kind::Formula, "ripgrep", None);
        assert_eq!(
            append_local_package(&path, &package).unwrap(),
            Append::Added
        );
        let first = std::fs::read(&path).unwrap();
        assert_eq!(
            append_local_package(&path, &package).unwrap(),
            Append::AlreadyPresent
        );
        assert_eq!(std::fs::read(&path).unwrap(), first);
        assert_eq!(std::fs::metadata(&path).unwrap().mode() & 0o777, 0o640);
        let doc = load(std::str::from_utf8(&first).unwrap()).unwrap();
        assert_eq!(doc.packages, vec![package]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_files_excludes_builtins_and_sorts_by_filename() {
        let dir = std::env::temp_dir().join(format!("macstrap-locals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for filename in ["z.yml", "a.yml"] {
            create_catalog(CreateCatalogInput {
                filename: filename.into(),
                title: filename.into(),
                description: None,
                location: dir.clone(),
            })
            .unwrap();
        }
        let entries = all_entries(&dir).unwrap();
        let names: Vec<&str> = local_files(&entries)
            .into_iter()
            .map(|file| file.path().unwrap().file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, ["a.yml", "z.yml"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn local_catalog_shows_in_all_entries() {
        let dir = std::env::temp_dir().join(format!("macstrap-entries-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = create_catalog(CreateCatalogInput {
            filename: "mine.yml".into(),
            title: "Mine".into(),
            description: None,
            location: dir.clone(),
        })
        .unwrap();
        let entries = all_entries(&dir).unwrap();
        assert!(entries.iter().any(|e| {
            e.file().origin == Origin::Local && e.file().path() == Some(path.as_path())
        }));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

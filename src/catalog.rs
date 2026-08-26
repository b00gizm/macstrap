use std::collections::{HashMap, HashSet};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Formula,
    Cask,
    Mas,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Origin {
    Builtin,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Self::Builtin => "builtin",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum CatalogId {
    CliEssentials,
    NodeEssentials,
    NodeFull,
    PythonEssentials,
    PythonFull,
    RustEssentials,
    RustFull,
}

pub struct CatalogFile {
    pub id: CatalogId,
    pub title: &'static str,
    pub origin: Origin,
    pub description: Option<&'static str>,
    pub required: bool,
    yaml: &'static str,
}

const FILES: &[CatalogFile] = &[
    CatalogFile {
        id: CatalogId::CliEssentials,
        title: "CLI essentials",
        origin: Origin::Builtin,
        description: Some("Absolute must-haves"),
        required: true,
        yaml: include_str!("../catalogs/cli-essentials.yml"),
    },
    CatalogFile {
        id: CatalogId::NodeEssentials,
        title: "Node essentials",
        origin: Origin::Builtin,
        description: Some("Minimal Node.js setup"),
        required: false,
        yaml: include_str!("../catalogs/node-essentials.yml"),
    },
    CatalogFile {
        id: CatalogId::NodeFull,
        title: "Node full",
        origin: Origin::Builtin,
        description: Some("Opinionated Node.js setup"),
        required: false,
        yaml: include_str!("../catalogs/node-full.yml"),
    },
    CatalogFile {
        id: CatalogId::PythonEssentials,
        title: "Python essentials",
        origin: Origin::Builtin,
        description: Some("Minimal Python setup"),
        required: false,
        yaml: include_str!("../catalogs/python-essentials.yml"),
    },
    CatalogFile {
        id: CatalogId::PythonFull,
        title: "Python full",
        origin: Origin::Builtin,
        description: Some("Opinionated Python setup"),
        required: false,
        yaml: include_str!("../catalogs/python-full.yml"),
    },
    CatalogFile {
        id: CatalogId::RustEssentials,
        title: "Rust essentials",
        origin: Origin::Builtin,
        description: Some("Minimal Rust setup"),
        required: false,
        yaml: include_str!("../catalogs/rust-essentials.yml"),
    },
    CatalogFile {
        id: CatalogId::RustFull,
        title: "Rust full",
        origin: Origin::Builtin,
        description: Some("Opinionated Rust setup"),
        required: false,
        yaml: include_str!("../catalogs/rust-full.yml"),
    },
];

pub fn files() -> &'static [CatalogFile] {
    FILES
}

pub fn default_loaded() -> HashSet<CatalogId> {
    FILES.iter().filter(|f| f.required).map(|f| f.id).collect()
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

#[derive(Debug, Deserialize)]
struct CatalogRow {
    kind: Kind,
    name: String,
    #[serde(default)]
    mas_id: Option<u64>,
    title: String,
    category: String,
    #[serde(default)]
    description: Option<String>,
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

pub fn load(yaml: &str) -> Result<Catalog, String> {
    let rows: Vec<CatalogRow> = serde_yaml::from_str(yaml).map_err(|e| format!("catalog: {e}"))?;
    Ok(rows.into_iter().map(Package::from).collect())
}

pub fn compose(loaded: &HashSet<CatalogId>) -> Result<Catalog, String> {
    let mut active = default_loaded();
    active.extend(loaded.iter().copied());
    let mut packages = Catalog::new();
    let mut seen = HashSet::new();
    for file in FILES {
        if !active.contains(&file.id) {
            continue;
        }
        for pkg in load(file.yaml)? {
            if seen.insert(pkg.id.clone()) {
                packages.push(pkg);
            }
        }
    }
    Ok(packages)
}

pub struct Merge {
    pub packages: Catalog,
    pub selection: Selection,
}

pub fn merge(catalog: &[Package], desired: &Desired) -> Merge {
    let mut packages = catalog.to_vec();
    let mut selection = Selection::new();
    for pkg in &packages {
        selection.insert(pkg.id.clone(), desired.contains_key(&pkg.id));
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
    fn yaml_description_is_optional() {
        let rows = load(
            r#"
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
        assert_eq!(
            rows[0].description.as_deref(),
            Some("Distributed version control")
        );
        assert_eq!(rows[1].description, None);
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
            let rows = load(file.yaml).unwrap();
            assert!(
                !rows.is_empty(),
                "{} must list at least one package",
                file.title
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
}

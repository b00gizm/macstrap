use std::collections::{HashMap, HashSet};

use serde::Deserialize;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Formula,
    Cask,
    Mas,
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
        }
    }
}

const CATALOG_YAML: &str = include_str!("../catalog.yaml");

pub fn load_embedded() -> Result<Catalog, String> {
    load(CATALOG_YAML)
}

pub fn load(yaml: &str) -> Result<Catalog, String> {
    let rows: Vec<CatalogRow> = serde_yaml::from_str(yaml).map_err(|e| format!("catalog: {e}"))?;
    Ok(rows.into_iter().map(Package::from).collect())
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
    Merge {
        packages,
        selection,
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
        }
    }

    #[test]
    fn load_embedded_catalog() {
        let catalog = load_embedded().unwrap();
        assert!(
            catalog
                .iter()
                .any(|p| p.name == "git" && p.kind == Kind::Formula)
        );
        assert!(
            catalog
                .iter()
                .any(|p| p.name == "visual-studio-code" && p.kind == Kind::Cask)
        );
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

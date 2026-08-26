use crate::catalog::{Desired, Kind, Package, PkgId};

pub struct Parsed {
    pub desired: Desired,
    pub skipped: Vec<String>,
}

pub fn parse(src: &str) -> Parsed {
    let mut desired = Desired::new();
    let mut skipped = Vec::new();
    for raw in src.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = quoted_after(line, "brew") {
            let pkg = package(Kind::Formula, name, None, name);
            desired.insert(pkg.id.clone(), pkg);
            continue;
        }
        if let Some(name) = quoted_after(line, "cask") {
            let pkg = package(Kind::Cask, name, None, name);
            desired.insert(pkg.id.clone(), pkg);
            continue;
        }
        if let Some((title, id)) = parse_mas(line) {
            let pkg = package(Kind::Mas, title, Some(id), title);
            desired.insert(pkg.id.clone(), pkg);
            continue;
        }
        skipped.push(line.to_string());
    }
    Parsed { desired, skipped }
}

fn package(kind: Kind, name: &str, mas_id: Option<u64>, title: &str) -> Package {
    Package {
        id: PkgId::new(kind, name, mas_id),
        kind,
        name: name.to_string(),
        mas_id,
        title: title.to_string(),
        category: "Brewfile".to_string(),
        description: None,
    }
}

fn quoted_after<'a>(line: &'a str, verb: &str) -> Option<&'a str> {
    let rest = line.strip_prefix(verb)?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(&rest[..end])
}

fn parse_mas(line: &str) -> Option<(&str, u64)> {
    let rest = line.strip_prefix("mas")?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let title_end = rest.find('"')?;
    let title = &rest[..title_end];
    let after = rest[title_end + 1..].trim();
    let after = after.strip_prefix(',')?.trim_start();
    let after = after.strip_prefix("id:")?.trim_start();
    let id: u64 = after
        .split(|c: char| !c.is_ascii_digit())
        .next()?
        .parse()
        .ok()?;
    Some((title, id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_testdata_brewfile() {
        let parsed = parse(include_str!("../testdata/Brewfile"));
        assert!(
            parsed
                .desired
                .contains_key(&PkgId::new(Kind::Formula, "git", None))
        );
        assert!(
            parsed
                .desired
                .contains_key(&PkgId::new(Kind::Cask, "visual-studio-code", None))
        );
        assert!(parsed.desired.contains_key(&PkgId::new(
            Kind::Mas,
            "Fantastical",
            Some(975937182)
        )));
        assert_eq!(parsed.desired.len(), 7);
        assert_eq!(parsed.skipped.len(), 2);
        assert!(parsed.skipped.iter().any(|l| l.starts_with("tap ")));
        assert!(parsed.skipped.iter().any(|l| l.starts_with("vscode ")));
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let parsed = parse("# hi\n\nbrew \"wget\"\n");
        assert_eq!(parsed.desired.len(), 1);
        assert!(parsed.skipped.is_empty());
    }
}

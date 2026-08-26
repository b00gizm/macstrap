use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};

use crate::brewfile;
use crate::catalog::{self, CatalogId, Desired, Kind, Observed, Package, Selection};
use crate::ensure::{self, Error, Host, Outcome};

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"), version, about = env!("CARGO_PKG_DESCRIPTION"))]
pub struct Opts {
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub brewfile: Option<PathBuf>,
    #[arg(long)]
    pub essentials_only: bool,
}

enum Page {
    Pick,
    Catalogs,
    Confirm,
}

enum PickAction {
    Continue,
    Catalogs,
    Confirm,
    Abort,
}

enum CatalogsAction {
    Continue,
    Done,
    Abort,
}

pub struct CatalogList {
    pub packages: Vec<Package>,
    pub selection: Selection,
    pub observed: Observed,
    pub loaded: HashSet<CatalogId>,
    pub desired: Desired,
    pub cursor: usize,
    pub catalog_cursor: usize,
    pub descriptions: HashMap<catalog::PkgId, String>,
    pub filter: String,
    pub filtering: bool,
}

impl CatalogList {
    pub fn visible(&self) -> Vec<usize> {
        let q = self.filter.to_ascii_lowercase();
        self.packages
            .iter()
            .enumerate()
            .filter(|(_, pkg)| {
                if q.is_empty() {
                    return true;
                }
                pkg.title.to_ascii_lowercase().contains(&q)
                    || pkg.name.to_ascii_lowercase().contains(&q)
                    || pkg.category.to_ascii_lowercase().contains(&q)
                    || pkg
                        .description
                        .as_deref()
                        .is_some_and(|d| d.to_ascii_lowercase().contains(&q))
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn current(&self) -> Option<usize> {
        self.visible().get(self.cursor).copied()
    }

    pub fn toggle(&mut self) {
        if let Some(i) = self.current() {
            let id = &self.packages[i].id;
            let on = self.selection.get(id).copied().unwrap_or(false);
            self.selection.insert(id.clone(), !on);
        }
    }

    pub fn select_all(&mut self) {
        for i in self.visible() {
            self.selection.insert(self.packages[i].id.clone(), true);
        }
    }

    pub fn select_none(&mut self) {
        for i in self.visible() {
            self.selection.insert(self.packages[i].id.clone(), false);
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, len as isize - 1) as usize;
    }

    pub fn reload(&mut self) {
        let catalog = catalog::compose(&self.loaded).expect("embedded catalogs parse");
        let merged = catalog::merge(&catalog, &self.desired);
        let old = std::mem::take(&mut self.selection);
        self.packages = merged.packages;
        self.selection = merged.selection;
        for (id, on) in old {
            if self.selection.contains_key(&id) {
                self.selection.insert(id, on);
            }
        }
        catalog::apply_descriptions(&mut self.packages, &self.descriptions);
        let vis = self.visible().len();
        self.cursor = if vis == 0 {
            0
        } else {
            self.cursor.min(vis - 1)
        };
    }

    pub fn toggle_catalog(&mut self) {
        let Some(file) = catalog::files().get(self.catalog_cursor) else {
            return;
        };
        if file.required {
            return;
        }
        if !self.loaded.remove(&file.id) {
            self.loaded.insert(file.id);
        }
        self.reload();
    }

    fn move_catalog_cursor(&mut self, delta: isize) {
        let len = catalog::files().len();
        if len == 0 {
            self.catalog_cursor = 0;
            return;
        }
        let next = self.catalog_cursor as isize + delta;
        self.catalog_cursor = next.clamp(0, len as isize - 1) as usize;
    }

    fn handle_catalog_key(&mut self, key: KeyEvent) -> CatalogsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => CatalogsAction::Done,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                CatalogsAction::Abort
            }
            KeyCode::Char('c') => CatalogsAction::Done,
            KeyCode::Char('q') => CatalogsAction::Abort,
            KeyCode::Char(' ') => {
                self.toggle_catalog();
                CatalogsAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_catalog_cursor(1);
                CatalogsAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_catalog_cursor(-1);
                CatalogsAction::Continue
            }
            _ => CatalogsAction::Continue,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> PickAction {
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.filtering = false;
                    self.filter.clear();
                    self.cursor = 0;
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.cursor = 0;
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.cursor = 0;
                }
                _ => {}
            }
            return PickAction::Continue;
        }
        match key.code {
            KeyCode::Char('q') => PickAction::Abort,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                PickAction::Abort
            }
            KeyCode::Char('c') => PickAction::Catalogs,
            KeyCode::Char('/') => {
                self.filtering = true;
                PickAction::Continue
            }
            KeyCode::Char(' ') => {
                self.toggle();
                PickAction::Continue
            }
            KeyCode::Char('a') => {
                self.select_all();
                PickAction::Continue
            }
            KeyCode::Char('n') => {
                self.select_none();
                PickAction::Continue
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_cursor(1);
                PickAction::Continue
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_cursor(-1);
                PickAction::Continue
            }
            KeyCode::Enter => PickAction::Confirm,
            _ => PickAction::Continue,
        }
    }
}

pub fn run(host: &impl Host, opts: Opts) -> Result<i32, Error> {
    if std::env::consts::OS != "macos" {
        return Err(Error::Message("This tool runs on macOS only.".into()));
    }

    println!("Essentials");
    let report = ensure::ensure_essentials(host);
    ensure::print_outcome("CLT", report.clt);
    ensure::print_outcome("Homebrew", report.brew);
    ensure::print_outcome("oh-my-zsh", report.omz);
    if matches!(report.clt, Outcome::Failed)
        || matches!(report.brew, Outcome::Failed)
        || matches!(report.omz, Outcome::Failed)
    {
        return Err(Error::Message("essentials failed".into()));
    }
    if opts.essentials_only {
        return Ok(0);
    }

    let loaded = catalog::default_loaded();
    let catalog = catalog::compose(&loaded).map_err(Error::Message)?;
    let (desired, skipped) = load_brewfile(opts.brewfile.as_ref())?;
    if !skipped.is_empty() {
        println!(
            "ignored {} Brewfile line(s) (tap, vscode, and other v1 skips)",
            skipped.len()
        );
    }
    let mut merged = catalog::merge(&catalog, &desired);
    let observed = host.installed()?;

    if opts.yes {
        return apply(host, &merged.packages, &merged.selection, &observed);
    }
    if !io::stdout().is_terminal() {
        return Err(Error::Message(
            "no TTY; pass --yes to apply the Brewfile selection".into(),
        ));
    }

    let all: HashSet<catalog::CatalogId> = catalog::files().iter().map(|f| f.id).collect();
    let mut universe = catalog::compose(&all).map_err(Error::Message)?;
    for pkg in desired.values() {
        if !universe.iter().any(|p| p.id == pkg.id) {
            universe.push(pkg.clone());
        }
    }
    let descriptions = host.descriptions(&universe)?;
    catalog::apply_descriptions(&mut merged.packages, &descriptions);

    let mut list = CatalogList {
        packages: merged.packages,
        selection: merged.selection,
        observed: observed.clone(),
        loaded,
        desired,
        cursor: 0,
        catalog_cursor: 0,
        descriptions,
        filter: String::new(),
        filtering: false,
    };
    let confirmed = pick(&mut list)?;
    if !confirmed {
        println!("aborted");
        return Ok(1);
    }
    apply(host, &list.packages, &list.selection, &list.observed)
}

fn load_brewfile(explicit: Option<&PathBuf>) -> Result<(catalog::Desired, Vec<String>), Error> {
    let path = match explicit {
        Some(p) => Some(p.clone()),
        None => default_brewfile(),
    };
    let Some(path) = path else {
        return Ok((catalog::Desired::new(), Vec::new()));
    };
    if !path.is_file() {
        return Err(Error::Message(format!(
            "Brewfile not found: {}",
            path.display()
        )));
    }
    let src = std::fs::read_to_string(&path)?;
    let parsed = brewfile::parse(&src);
    Ok((parsed.desired, parsed.skipped))
}

fn default_brewfile() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let plain = home.join("Brewfile");
    if plain.is_file() {
        return Some(plain);
    }
    let hidden = home.join(".Brewfile");
    if hidden.is_file() {
        return Some(hidden);
    }
    None
}

fn apply(
    host: &impl Host,
    packages: &[Package],
    selection: &Selection,
    observed: &Observed,
) -> Result<i32, Error> {
    println!("Apply");
    let mut failed = 0;
    let mut seen = observed.clone();
    let mas_id = catalog::PkgId::new(Kind::Formula, "mas", None);
    let needs_mas = catalog::pending(packages, selection, observed)
        .iter()
        .any(|p| p.kind == Kind::Mas);
    if needs_mas && !seen.contains(&mas_id) {
        let owned = Package {
            id: mas_id.clone(),
            kind: Kind::Formula,
            name: "mas".into(),
            mas_id: None,
            title: "mas".into(),
            category: "CLI".into(),
            description: None,
        };
        let mas = packages.iter().find(|p| p.id == mas_id).unwrap_or(&owned);
        let outcome = ensure::ensure_package(host, mas, &seen);
        ensure::print_outcome("mas", outcome);
        match outcome {
            Outcome::Failed => return Err(Error::Message("failed to install mas".into())),
            _ => {
                seen.insert(mas_id);
            }
        }
    }
    for pkg in catalog::pending(packages, selection, &seen) {
        if seen.contains(&pkg.id) {
            continue;
        }
        let outcome = ensure::ensure_package(host, pkg, &seen);
        ensure::print_outcome(&pkg.name, outcome);
        match outcome {
            Outcome::Applied | Outcome::Satisfied => {
                seen.insert(pkg.id.clone());
            }
            Outcome::Failed => failed += 1,
        }
    }
    if failed > 0 {
        return Err(Error::Message(format!("{failed} package(s) failed")));
    }
    Ok(0)
}

fn pick(list: &mut CatalogList) -> Result<bool, Error> {
    let mut terminal = ratatui::init();
    let result = pick_loop(&mut terminal, list);
    ratatui::restore();
    result
}

fn pick_loop(terminal: &mut DefaultTerminal, list: &mut CatalogList) -> Result<bool, Error> {
    let mut page = Page::Pick;
    loop {
        terminal
            .draw(|frame| match page {
                Page::Pick => draw_pick(frame, list),
                Page::Catalogs => draw_catalogs(frame, list),
                Page::Confirm => draw_confirm(frame, list),
            })
            .map_err(|e| Error::Message(e.to_string()))?;
        let Event::Key(key) = event::read().map_err(Error::from)? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match page {
            Page::Pick => match list.handle_key(key) {
                PickAction::Continue => {}
                PickAction::Catalogs => page = Page::Catalogs,
                PickAction::Confirm => page = Page::Confirm,
                PickAction::Abort => return Ok(false),
            },
            Page::Catalogs => match list.handle_catalog_key(key) {
                CatalogsAction::Continue => {}
                CatalogsAction::Done => page = Page::Pick,
                CatalogsAction::Abort => return Ok(false),
            },
            Page::Confirm => match key.code {
                KeyCode::Enter => return Ok(true),
                KeyCode::Esc => page = Page::Pick,
                KeyCode::Char('q') => return Ok(false),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(false);
                }
                _ => {}
            },
        }
    }
}

fn draw_pick(frame: &mut ratatui::Frame, list: &CatalogList) {
    let areas = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(frame.area());
    let visible = list.visible();
    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(vis_i, &i)| {
            let pkg = &list.packages[i];
            let checked = list.selection.get(&pkg.id).copied().unwrap_or(false);
            let mark = if checked { "[x]" } else { "[ ]" };
            let installed = if list.observed.contains(&pkg.id) {
                "  installed"
            } else {
                ""
            };
            let style = if vis_i == list.cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![Span::raw(format!(
                "{mark} {:<24} {:<12} {:<40}{installed}",
                pkg.title,
                pkg.category,
                pkg.description.as_deref().unwrap_or("")
            ))]))
            .style(style)
        })
        .collect();
    let title = if list.filtering {
        format!("Choose tools  /{}", list.filter)
    } else {
        "Choose tools".into()
    };
    let widget = List::new(items).block(Block::default().borders(Borders::ALL).title(title));
    frame.render_widget(widget, areas[0]);
    frame.render_widget(
        Paragraph::new("space toggle  a all  n none  / filter  c catalogs  enter confirm  q abort"),
        areas[1],
    );
}

fn draw_catalogs(frame: &mut ratatui::Frame, list: &CatalogList) {
    let areas = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(frame.area());
    let items: Vec<ListItem> = catalog::files()
        .iter()
        .enumerate()
        .map(|(i, file)| {
            let checked = list.loaded.contains(&file.id) || file.required;
            let mark = if checked { "[x]" } else { "[ ]" };
            let extra = if file.required { "  always on" } else { "" };
            let style = if i == list.catalog_cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(Line::from(vec![Span::raw(format!(
                "{mark} {:<20} {:<8} {:<32}{extra}",
                file.title,
                file.origin.label(),
                file.description.unwrap_or("")
            ))]))
            .style(style)
        })
        .collect();
    let widget = List::new(items).block(Block::default().borders(Borders::ALL).title("Catalogs"));
    frame.render_widget(widget, areas[0]);
    frame.render_widget(Paragraph::new("space load/unload  enter back"), areas[1]);
}

fn draw_confirm(frame: &mut ratatui::Frame, list: &CatalogList) {
    let n = catalog::pending(&list.packages, &list.selection, &list.observed).len();
    let text = format!("{n} package(s) to install\nenter apply   esc back");
    frame.render_widget(
        Paragraph::new(text).block(Block::default().borders(Borders::ALL).title("Apply")),
        frame.area(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Kind, Package, PkgId};

    fn pkg(name: &str) -> Package {
        Package {
            id: PkgId::new(Kind::Formula, name, None),
            kind: Kind::Formula,
            name: name.into(),
            mas_id: None,
            title: name.into(),
            category: "CLI".into(),
            description: None,
        }
    }

    fn list() -> CatalogList {
        let packages = vec![pkg("git"), pkg("ripgrep"), pkg("fzf")];
        let mut selection = Selection::new();
        selection.insert(packages[0].id.clone(), true);
        selection.insert(packages[1].id.clone(), false);
        selection.insert(packages[2].id.clone(), false);
        CatalogList {
            packages,
            selection,
            observed: Observed::new(),
            loaded: catalog::default_loaded(),
            desired: Desired::new(),
            cursor: 0,
            catalog_cursor: 0,
            descriptions: HashMap::new(),
            filter: String::new(),
            filtering: false,
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn q_aborts_picker() {
        let mut l = list();
        assert!(matches!(l.handle_key(key('q')), PickAction::Abort));
    }

    #[test]
    fn c_opens_catalogs() {
        let mut l = list();
        assert!(matches!(l.handle_key(key('c')), PickAction::Catalogs));
    }

    fn live_list() -> CatalogList {
        let loaded = catalog::default_loaded();
        let catalog = catalog::compose(&loaded).unwrap();
        let merged = catalog::merge(&catalog, &Desired::new());
        CatalogList {
            packages: merged.packages,
            selection: merged.selection,
            observed: Observed::new(),
            loaded,
            desired: Desired::new(),
            cursor: 0,
            catalog_cursor: 0,
            descriptions: HashMap::new(),
            filter: String::new(),
            filtering: false,
        }
    }

    fn catalog_index(id: CatalogId) -> usize {
        catalog::files().iter().position(|f| f.id == id).unwrap()
    }

    #[test]
    fn load_and_unload_refreshes_packages() {
        let mut l = live_list();
        assert!(!l.packages.iter().any(|p| p.name == "node"));

        l.catalog_cursor = catalog_index(CatalogId::NodeEssentials);
        l.toggle_catalog();
        assert!(l.packages.iter().any(|p| p.name == "node"));
        assert!(l.packages.iter().any(|p| p.name == "git"));

        l.toggle_catalog();
        assert!(!l.packages.iter().any(|p| p.name == "node"));
        assert!(l.packages.iter().any(|p| p.name == "git"));
    }

    #[test]
    fn required_catalog_stays_loaded() {
        let mut l = live_list();
        l.catalog_cursor = catalog_index(CatalogId::CliEssentials);
        l.toggle_catalog();
        assert!(l.loaded.contains(&CatalogId::CliEssentials));
        assert!(l.packages.iter().any(|p| p.name == "git"));
    }

    #[test]
    fn catalogs_space_reloads_list() {
        let mut l = live_list();
        l.catalog_cursor = catalog_index(CatalogId::NodeEssentials);
        assert!(matches!(
            l.handle_catalog_key(key(' ')),
            CatalogsAction::Continue
        ));
        assert!(l.packages.iter().any(|p| p.name == "node"));
        assert!(matches!(
            l.handle_catalog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            CatalogsAction::Done
        ));
    }

    #[test]
    fn reload_applies_cached_brew_descriptions() {
        let mut l = live_list();
        l.descriptions.insert(
            PkgId::new(Kind::Formula, "node", None),
            "JavaScript runtime".into(),
        );
        l.catalog_cursor = catalog_index(CatalogId::NodeEssentials);
        l.toggle_catalog();
        let node = l.packages.iter().find(|p| p.name == "node").unwrap();
        assert_eq!(node.description.as_deref(), Some("JavaScript runtime"));
    }

    #[test]
    fn reload_keeps_user_toggles() {
        let mut l = live_list();
        let git = PkgId::new(Kind::Formula, "git", None);
        l.selection.insert(git.clone(), true);
        l.catalog_cursor = catalog_index(CatalogId::NodeEssentials);
        l.toggle_catalog();
        assert!(l.selection[&git]);
        assert!(!l.selection[&PkgId::new(Kind::Formula, "node", None)]);
    }

    #[test]
    fn toggle_and_select_all_none() {
        let mut l = list();
        l.toggle();
        assert!(!l.selection[&l.packages[0].id]);
        l.select_all();
        assert!(l.selection.values().all(|v| *v));
        l.select_none();
        assert!(l.selection.values().all(|v| !*v));
    }

    #[test]
    fn filter_limits_select_all() {
        let mut l = list();
        l.filter = "rip".into();
        l.select_all();
        assert!(l.selection[&PkgId::new(Kind::Formula, "ripgrep", None)]);
        assert!(!l.selection[&PkgId::new(Kind::Formula, "fzf", None)]);
    }

    #[test]
    fn home_brewfile_preselects_every_entry() {
        let Ok(home) = std::env::var("HOME") else {
            return;
        };
        let path = PathBuf::from(home).join("Brewfile");
        let Ok(src) = std::fs::read_to_string(path) else {
            return;
        };
        let parsed = brewfile::parse(&src);
        let catalog = catalog::compose(&catalog::default_loaded()).unwrap();
        let merged = catalog::merge(&catalog, &parsed.desired);
        for id in parsed.desired.keys() {
            assert!(
                merged.selection.get(id).copied().unwrap_or(false),
                "Brewfile id not preselected: {id:?}"
            );
        }
    }

    #[test]
    fn brewfile_preselect_matches_testdata() {
        let catalog = catalog::compose(&catalog::default_loaded()).unwrap();
        let parsed = brewfile::parse(include_str!("../testdata/Brewfile"));
        let merged = catalog::merge(&catalog, &parsed.desired);
        assert!(merged.selection[&PkgId::new(Kind::Formula, "git", None)]);
        assert!(merged.selection[&PkgId::new(Kind::Cask, "visual-studio-code", None)]);
        assert!(merged.selection[&PkgId::new(Kind::Mas, "Yoink", Some(457622435))]);
        assert!(!merged.selection[&PkgId::new(Kind::Formula, "fzf", None)]);
    }
}

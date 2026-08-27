use std::collections::{HashMap, HashSet};
use std::io::{self, IsTerminal};
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseEventKind,
};
use crossterm::execute;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::DefaultTerminal;

use crate::brewfile;
use crate::catalog::{
    self, CatalogEntry, CatalogId, CreateCatalogInput, Desired, Kind, Observed, Package, PkgId,
    Selection,
};
use crate::ensure::{self, Error, Host, Outcome};

#[derive(Debug, Parser)]
#[command(name = env!("CARGO_PKG_NAME"), version, about = env!("CARGO_PKG_DESCRIPTION"))]
pub struct Opts {
    #[command(subcommand)]
    pub command: Option<Command>,
    #[arg(long)]
    pub yes: bool,
    #[arg(long)]
    pub brewfile: Option<PathBuf>,
    #[arg(long)]
    pub essentials_only: bool,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Manage catalogs
    Catalog {
        #[command(subcommand)]
        action: CatalogCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum CatalogCommand {
    /// Create a custom catalog file
    Create,
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
    Create,
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
    pub catalog_entries: Vec<CatalogEntry>,
    pub config_dir: PathBuf,
    pub facts: HashMap<catalog::PkgId, catalog::BrewFacts>,
    pub filter: String,
    pub filtering: bool,
    pub show_all_installed: bool,
    pub catalog_pkg_ids: HashSet<PkgId>,
    list_state: ListState,
    catalog_list_state: ListState,
    pick_rows: usize,
    catalog_rows: usize,
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
            if on && catalog::is_protected(id) {
                return;
            }
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
            let id = &self.packages[i].id;
            self.selection.insert(id.clone(), catalog::is_protected(id));
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.set_pick_cursor(0);
            return;
        }
        let next = self.cursor as isize + delta;
        self.set_pick_cursor(next.clamp(0, len as isize - 1) as usize);
    }

    fn set_pick_cursor(&mut self, index: usize) {
        let len = self.visible().len();
        if len == 0 {
            self.cursor = 0;
            self.list_state.select(None);
            *self.list_state.offset_mut() = 0;
            return;
        }
        self.cursor = index.min(len - 1);
        self.list_state.select(Some(self.cursor));
    }

    fn max_pick_offset(&self) -> usize {
        self.visible().len().saturating_sub(self.pick_rows.max(1))
    }

    fn scroll_pick(&mut self, delta: isize) {
        let len = self.visible().len();
        if len == 0 {
            self.set_pick_cursor(0);
            return;
        }
        let next = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        self.set_pick_cursor(next);
        *self.list_state.offset_mut() = next.min(self.max_pick_offset());
    }

    fn nudge_pick(&mut self, delta: isize) {
        self.move_cursor(delta);
        let max_off = self.max_pick_offset();
        let off = (self.list_state.offset() as isize + delta).clamp(0, max_off as isize) as usize;
        *self.list_state.offset_mut() = off;
    }

    fn move_catalog_cursor(&mut self, delta: isize) {
        let len = self.catalog_entries.len();
        if len == 0 {
            self.set_catalog_cursor(0);
            return;
        }
        let next = self.catalog_cursor as isize + delta;
        self.set_catalog_cursor(next.clamp(0, len as isize - 1) as usize);
    }

    fn set_catalog_cursor(&mut self, index: usize) {
        let len = self.catalog_entries.len();
        if len == 0 {
            self.catalog_cursor = 0;
            self.catalog_list_state.select(None);
            *self.catalog_list_state.offset_mut() = 0;
            return;
        }
        self.catalog_cursor = index.min(len - 1);
        self.catalog_list_state.select(Some(self.catalog_cursor));
    }

    fn max_catalog_offset(&self) -> usize {
        self.catalog_entries
            .len()
            .saturating_sub(self.catalog_rows.max(1))
    }

    fn scroll_catalogs(&mut self, delta: isize) {
        let len = self.catalog_entries.len();
        if len == 0 {
            self.set_catalog_cursor(0);
            return;
        }
        let next = (self.catalog_cursor as isize + delta).clamp(0, len as isize - 1) as usize;
        self.set_catalog_cursor(next);
        *self.catalog_list_state.offset_mut() = next.min(self.max_catalog_offset());
    }

    fn nudge_catalogs(&mut self, delta: isize) {
        self.move_catalog_cursor(delta);
        let max_off = self.max_catalog_offset();
        let off =
            (self.catalog_list_state.offset() as isize + delta).clamp(0, max_off as isize) as usize;
        *self.catalog_list_state.offset_mut() = off;
    }

    pub fn reload(&mut self) {
        let catalog = catalog::compose(&self.loaded).expect("embedded catalogs parse");
        self.catalog_pkg_ids = catalog.iter().map(|p| p.id.clone()).collect();
        let universe = catalog::compose_all().expect("embedded catalogs parse");
        let mut merged = catalog::merge(&catalog, &self.desired);
        catalog::include_observed(
            &mut merged,
            &self.observed,
            &universe,
            self.show_all_installed,
        );
        let old = std::mem::take(&mut self.selection);
        self.packages = merged.packages;
        self.selection = merged.selection;
        for (id, on) in &old {
            if self.selection.contains_key(id) {
                self.selection.insert(id.clone(), *on);
            }
        }
        catalog::preselect_installed(&mut self.selection, &self.observed, Some(&old));
        catalog::apply_facts(&mut self.packages, &self.facts);
        self.set_pick_cursor(self.cursor);
    }

    pub fn toggle_show_all_installed(&mut self) {
        self.show_all_installed = !self.show_all_installed;
        self.reload();
    }

    pub fn persist_catalogs(&self) -> Result<(), Error> {
        catalog::save_persisted_catalogs(&self.config_dir, &self.loaded).map_err(Error::Message)
    }

    pub fn refresh_catalog_entries(&mut self) -> Result<(), Error> {
        self.catalog_entries = catalog::all_entries(&self.config_dir).map_err(Error::Message)?;
        self.set_catalog_cursor(self.catalog_cursor);
        Ok(())
    }

    pub fn toggle_catalog(&mut self) {
        let Some(entry) = self.catalog_entries.get(self.catalog_cursor) else {
            return;
        };
        let file = entry.file();
        if file.required {
            return;
        }
        if !self.loaded.remove(&file.id) {
            self.loaded.insert(file.id.clone());
        }
        self.reload();
        let _ = self.persist_catalogs();
    }

    fn handle_catalog_key(&mut self, key: KeyEvent) -> CatalogsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => CatalogsAction::Done,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                CatalogsAction::Abort
            }
            KeyCode::Char('c') => CatalogsAction::Done,
            KeyCode::Char('n') => CatalogsAction::Create,
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
            KeyCode::PageDown => {
                self.scroll_catalogs(self.catalog_rows.max(1) as isize);
                CatalogsAction::Continue
            }
            KeyCode::PageUp => {
                self.scroll_catalogs(-(self.catalog_rows.max(1) as isize));
                CatalogsAction::Continue
            }
            KeyCode::Home => {
                self.scroll_catalogs(-(self.catalog_cursor as isize));
                CatalogsAction::Continue
            }
            KeyCode::End => {
                let last = self.catalog_entries.len().saturating_sub(1);
                self.scroll_catalogs(last as isize - self.catalog_cursor as isize);
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
                    self.reset_pick_view();
                }
                KeyCode::Enter => self.filtering = false,
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.reset_pick_view();
                }
                KeyCode::Char(c) => {
                    self.filter.push(c);
                    self.reset_pick_view();
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
            KeyCode::Char('o') => {
                self.toggle_show_all_installed();
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
            KeyCode::PageDown => {
                self.scroll_pick(self.pick_rows.max(1) as isize);
                PickAction::Continue
            }
            KeyCode::PageUp => {
                self.scroll_pick(-(self.pick_rows.max(1) as isize));
                PickAction::Continue
            }
            KeyCode::Home => {
                self.scroll_pick(-(self.cursor as isize));
                PickAction::Continue
            }
            KeyCode::End => {
                let last = self.visible().len().saturating_sub(1);
                self.scroll_pick(last as isize - self.cursor as isize);
                PickAction::Continue
            }
            KeyCode::Enter => PickAction::Confirm,
            _ => PickAction::Continue,
        }
    }

    fn reset_pick_view(&mut self) {
        self.set_pick_cursor(0);
        *self.list_state.offset_mut() = 0;
    }
}

pub fn run(host: &impl Host, opts: Opts) -> Result<i32, Error> {
    if std::env::consts::OS != "macos" {
        return Err(Error::Message("This tool runs on macOS only.".into()));
    }

    if let Some(Command::Catalog {
        action: CatalogCommand::Create,
    }) = opts.command
    {
        let path = run_catalog_create_interactive()?;
        println!("Created {}", path.display());
        return Ok(0);
    }

    ensure::print_banner();
    ensure::print_section("Essentials");
    let report = ensure::ensure_essentials(host);
    ensure::print_outcome("Command Line Tools", report.clt);
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

    ensure::print_section("CLI essentials");
    ensure::ensure_cli_essentials(host)?;

    let config_dir = catalog::default_config_dir();
    let loaded = catalog::load_persisted_catalogs(&config_dir);
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
    let universe = catalog::compose_all().map_err(Error::Message)?;
    catalog::include_observed(&mut merged, &observed, &universe, false);
    catalog::preselect_installed(&mut merged.selection, &observed, None);

    if opts.yes {
        return apply(host, &merged.packages, &merged.selection, &observed);
    }
    if !io::stdout().is_terminal() {
        return Err(Error::Message(
            "no TTY; pass --yes to apply the Brewfile selection".into(),
        ));
    }

    let facts = host.brew_facts(&universe)?;
    catalog::apply_facts(&mut merged.packages, &facts);

    let catalog_pkg_ids = catalog.iter().map(|p| p.id.clone()).collect();
    let mut list = CatalogList {
        packages: merged.packages,
        selection: merged.selection,
        observed: observed.clone(),
        loaded,
        catalog_pkg_ids,
        desired,
        cursor: 0,
        catalog_cursor: 0,
        catalog_entries: catalog::all_entries(&config_dir).map_err(Error::Message)?,
        config_dir,
        facts,
        filter: String::new(),
        filtering: false,
        show_all_installed: false,
        list_state: ListState::default(),
        catalog_list_state: ListState::default(),
        pick_rows: 10,
        catalog_rows: 10,
    };
    loop {
        let confirmed = pick(&mut list)?;
        if !confirmed {
            ensure::print_goodbye();
            return Ok(1);
        }
        if let Err(err) = apply(host, &list.packages, &list.selection, &list.observed) {
            eprintln!("{err}");
        }
        refresh_list(host, &mut list)?;
    }
}

fn refresh_list(host: &impl Host, list: &mut CatalogList) -> Result<(), Error> {
    list.observed = host.installed()?;
    let universe = catalog::compose_all().map_err(Error::Message)?;
    list.facts = host.brew_facts(&universe)?;
    list.reload();
    Ok(())
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
    ensure::print_section("Apply");
    let mut failed = 0;
    let mut seen = observed.clone();
    for pkg in catalog::pending_uninstall(packages, selection, &seen) {
        let outcome = ensure::remove_package(host, pkg, &seen);
        ensure::print_outcome(&pkg.name, outcome);
        match outcome {
            Outcome::Removed | Outcome::Satisfied => {
                seen.remove(&pkg.id);
            }
            Outcome::Failed => failed += 1,
            Outcome::Applied => {}
        }
    }
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
            available: None,
            installed_version: None,
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
            Outcome::Removed => {}
        }
    }
    if failed > 0 {
        return Err(Error::Message(format!("{failed} package(s) failed")));
    }
    Ok(0)
}

fn pick(list: &mut CatalogList) -> Result<bool, Error> {
    let mut terminal = ratatui::init();
    execute!(io::stdout(), EnableMouseCapture).map_err(Error::from)?;
    let result = pick_loop(&mut terminal, list);
    let _ = execute!(io::stdout(), DisableMouseCapture);
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
        match event::read().map_err(Error::from)? {
            Event::Key(key) if actionable_key(&key) => match page {
                Page::Pick => match list.handle_key(key) {
                    PickAction::Continue => {}
                    PickAction::Catalogs => page = Page::Catalogs,
                    PickAction::Confirm => page = Page::Confirm,
                    PickAction::Abort => return Ok(false),
                },
                Page::Catalogs => match list.handle_catalog_key(key) {
                    CatalogsAction::Continue => {}
                    CatalogsAction::Done => {
                        if let Err(err) = list.persist_catalogs() {
                            eprintln!("{err}");
                        }
                        page = Page::Pick;
                    }
                    CatalogsAction::Create => {
                        let _ = execute!(io::stdout(), DisableMouseCapture);
                        ratatui::restore();
                        match run_catalog_create_interactive() {
                            Ok(path) => {
                                list.loaded.insert(CatalogId::Local(path));
                                list.refresh_catalog_entries()?;
                                list.reload();
                                let _ = list.persist_catalogs();
                            }
                            Err(err) => eprintln!("{err}"),
                        }
                        *terminal = ratatui::init();
                        let _ = execute!(io::stdout(), EnableMouseCapture);
                    }
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
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::ScrollDown => match page {
                    Page::Pick => list.nudge_pick(1),
                    Page::Catalogs => list.nudge_catalogs(1),
                    Page::Confirm => {}
                },
                MouseEventKind::ScrollUp => match page {
                    Page::Pick => list.nudge_pick(-1),
                    Page::Catalogs => list.nudge_catalogs(-1),
                    Page::Confirm => {}
                },
                _ => {}
            },
            Event::Resize(_, _) => {}
            _ => {}
        }
    }
}

fn actionable_key(key: &KeyEvent) -> bool {
    matches!(key.kind, KeyEventKind::Press | KeyEventKind::Repeat)
}

fn cell(s: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut chars: Vec<char> = s.chars().collect();
    if chars.len() > width {
        chars.truncate(width);
        chars[width - 1] = '…';
    }
    let mut out: String = chars.into_iter().collect();
    while out.chars().count() < width {
        out.push(' ');
    }
    out
}

fn package_row_style(pkg: &Package, list: &CatalogList) -> Style {
    let mut style = Style::default();
    if list.observed.contains(&pkg.id) {
        style = if list.catalog_pkg_ids.contains(&pkg.id) {
            style.fg(Color::Green)
        } else {
            style.fg(Color::DarkGray)
        };
    }
    style
}

fn draw_pick(frame: &mut ratatui::Frame, list: &mut CatalogList) {
    let areas = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(frame.area());
    list.pick_rows = areas[0].height.saturating_sub(2).max(1) as usize;
    let inner = areas[0].width.saturating_sub(2) as usize;
    let desc_w = inner.saturating_sub(72);
    let visible = list.visible();
    let items: Vec<ListItem> = visible
        .iter()
        .enumerate()
        .map(|(_, &i)| {
            let pkg = &list.packages[i];
            let checked = list.selection.get(&pkg.id).copied().unwrap_or(false);
            let mark = if checked { "[x]" } else { "[ ]" };
            let style = package_row_style(pkg, list);
            ListItem::new(Line::from(vec![Span::raw(format!(
                "{mark} {} {} {} {} {}",
                cell(&pkg.title, 24),
                cell(&pkg.category, 12),
                cell(pkg.description.as_deref().unwrap_or(""), desc_w),
                cell(pkg.installed_version.as_deref().unwrap_or(""), 14),
                cell(pkg.available.as_deref().unwrap_or(""), 14)
            ))]))
            .style(style)
        })
        .collect();
    let title = if list.filtering {
        format!("Choose tools  /{}", list.filter)
    } else if list.show_all_installed {
        "Choose tools · all installed".into()
    } else {
        "Choose tools · catalog".into()
    };
    let widget = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    list.list_state.select(if visible.is_empty() {
        None
    } else {
        Some(list.cursor)
    });
    frame.render_stateful_widget(widget, areas[0], &mut list.list_state);
    frame.render_widget(
        Paragraph::new(
            "space toggle  a all  n none  o all installed  / filter  c catalogs  enter confirm  q abort",
        ),
        areas[1],
    );
}

fn draw_catalogs(frame: &mut ratatui::Frame, list: &mut CatalogList) {
    let areas = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).split(frame.area());
    list.catalog_rows = areas[0].height.saturating_sub(2).max(1) as usize;
    let inner = areas[0].width.saturating_sub(2) as usize;
    let desc_w = inner.saturating_sub(44);
    let items: Vec<ListItem> = list
        .catalog_entries
        .iter()
        .enumerate()
        .map(|(_, entry)| {
            let file = entry.file();
            let checked = list.loaded.contains(&file.id) || file.required;
            let mark = if checked { "[x]" } else { "[ ]" };
            let extra = if file.required { "always on" } else { "" };
            let doc = file.doc().expect("embedded catalogs parse");
            ListItem::new(Line::from(vec![Span::raw(format!(
                "{mark} {} {} {} {}",
                cell(&doc.title, 20),
                cell(file.origin.label(), 8),
                cell(doc.description.as_deref().unwrap_or(""), desc_w),
                cell(extra, 9)
            ))]))
        })
        .collect();
    let widget = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Catalogs"))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    list.catalog_list_state
        .select(if list.catalog_entries.is_empty() {
            None
        } else {
            Some(list.catalog_cursor)
        });
    frame.render_stateful_widget(widget, areas[0], &mut list.catalog_list_state);
    frame.render_widget(
        Paragraph::new("space load/unload  n new  enter back"),
        areas[1],
    );
}

fn run_catalog_create_interactive() -> Result<PathBuf, Error> {
    let default_dir = catalog::default_config_dir();
    let filename = prompt_line("File name", None)?;
    if filename.trim().is_empty() {
        return Err(Error::Message("file name is required".into()));
    }
    let default_title = catalog::infer_title_from_filename(&filename);
    let title = prompt_line("Name", Some(&default_title))?;
    let description = prompt_line("Description", Some(""))?;
    let description = if description.trim().is_empty() {
        None
    } else {
        Some(description)
    };
    let location = prompt_line("Location", Some(&default_dir.display().to_string()))?;
    let location = PathBuf::from(location);
    catalog::create_catalog(CreateCatalogInput {
        filename,
        title,
        description,
        location,
    })
    .map_err(Error::Message)
}

fn prompt_line(label: &str, default: Option<&str>) -> Result<String, Error> {
    use std::io::Write;
    match default {
        Some(d) => print!("{label} [{d}]: "),
        None => print!("{label}: "),
    }
    io::stdout().flush().map_err(Error::from)?;
    let mut line = String::new();
    io::stdin().read_line(&mut line).map_err(Error::from)?;
    let line = line.trim_end_matches(['\n', '\r']).to_string();
    if line.is_empty() {
        return Ok(default.unwrap_or("").to_string());
    }
    Ok(line)
}

fn draw_confirm(frame: &mut ratatui::Frame, list: &CatalogList) {
    let install = catalog::pending(&list.packages, &list.selection, &list.observed).len();
    let uninstall =
        catalog::pending_uninstall(&list.packages, &list.selection, &list.observed).len();
    let text = format!("{install} to install, {uninstall} to uninstall\nenter apply   esc back");
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
            available: None,
            installed_version: None,
        }
    }

    fn list() -> CatalogList {
        let packages = vec![pkg("ripgrep"), pkg("node"), pkg("bun")];
        let mut selection = Selection::new();
        selection.insert(packages[0].id.clone(), true);
        selection.insert(packages[1].id.clone(), false);
        selection.insert(packages[2].id.clone(), false);
        let config_dir = catalog::default_config_dir();
        let loaded = catalog::default_loaded();
        let catalog_pkg_ids = catalog::compose(&loaded)
            .unwrap()
            .into_iter()
            .map(|p| p.id)
            .collect();
        CatalogList {
            packages,
            selection,
            observed: Observed::new(),
            loaded,
            catalog_pkg_ids,
            desired: Desired::new(),
            cursor: 0,
            catalog_cursor: 0,
            catalog_entries: catalog::all_entries(&config_dir).unwrap(),
            config_dir,
            facts: HashMap::new(),
            filter: String::new(),
            filtering: false,
            show_all_installed: false,
            list_state: ListState::default(),
            catalog_list_state: ListState::default(),
            pick_rows: 10,
            catalog_rows: 10,
        }
    }

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn cell_clips_and_pads() {
        assert_eq!(cell("hi", 4), "hi  ");
        assert_eq!(cell("toolong", 4), "too…");
        assert_eq!(cell("ab", 0), "");
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
        let config_dir = catalog::default_config_dir();
        let catalog_pkg_ids = catalog.iter().map(|p| p.id.clone()).collect();
        CatalogList {
            packages: merged.packages,
            selection: merged.selection,
            observed: Observed::new(),
            loaded,
            catalog_pkg_ids,
            desired: Desired::new(),
            cursor: 0,
            catalog_cursor: 0,
            catalog_entries: catalog::all_entries(&config_dir).unwrap(),
            config_dir,
            facts: HashMap::new(),
            filter: String::new(),
            filtering: false,
            show_all_installed: false,
            list_state: ListState::default(),
            catalog_list_state: ListState::default(),
            pick_rows: 10,
            catalog_rows: 10,
        }
    }

    fn catalog_index(id: CatalogId) -> usize {
        let config_dir = catalog::default_config_dir();
        catalog::all_entries(&config_dir)
            .unwrap()
            .iter()
            .position(|e| e.file().id == id)
            .unwrap()
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
    fn reload_applies_cached_brew_facts() {
        let mut l = live_list();
        l.facts.insert(
            PkgId::new(Kind::Formula, "node", None),
            catalog::BrewFacts {
                description: Some("JavaScript runtime".into()),
                available: Some("24.0.0".into()),
                installed: Some("22.0.0".into()),
            },
        );
        l.catalog_cursor = catalog_index(CatalogId::NodeEssentials);
        l.toggle_catalog();
        let node = l.packages.iter().find(|p| p.name == "node").unwrap();
        assert_eq!(node.description.as_deref(), Some("JavaScript runtime"));
        assert_eq!(node.available.as_deref(), Some("24.0.0"));
        assert_eq!(node.installed_version.as_deref(), Some("22.0.0"));
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
        assert!(!l.selection[&PkgId::new(Kind::Formula, "bun", None)]);
    }

    #[test]
    fn refresh_list_syncs_observed_and_selection() {
        let host = ensure::FakeHost::default();
        let mut l = live_list();
        let fd = PkgId::new(Kind::Formula, "fd", None);
        l.selection.insert(fd.clone(), true);
        host.installed.borrow_mut().insert(fd.clone());
        refresh_list(&host, &mut l).unwrap();
        assert!(l.observed.contains(&fd));
        assert!(l.selection.get(&fd).copied().unwrap_or(false));
    }

    #[test]
    fn apply_uninstalls_deselected_observed() {
        let host = ensure::FakeHost::default();
        let git = pkg("git");
        let rg = pkg("ripgrep");
        host.installed.borrow_mut().insert(git.id.clone());
        host.installed.borrow_mut().insert(rg.id.clone());
        let packages = vec![git.clone(), rg.clone()];
        let mut selection = Selection::new();
        selection.insert(git.id.clone(), false);
        selection.insert(rg.id.clone(), true);
        let observed = host.installed.borrow().clone();
        apply(&host, &packages, &selection, &observed).unwrap();
        assert!(host.uninstalls.borrow().is_empty());
        assert!(host.installs.borrow().is_empty());
    }

    #[test]
    fn apply_uninstalls_non_essential_only() {
        let host = ensure::FakeHost::default();
        let git = pkg("git");
        let rg = pkg("ripgrep");
        host.installed.borrow_mut().insert(git.id.clone());
        host.installed.borrow_mut().insert(rg.id.clone());
        let packages = vec![git, rg.clone()];
        let mut selection = Selection::new();
        selection.insert(packages[0].id.clone(), true);
        selection.insert(rg.id.clone(), false);
        let observed = host.installed.borrow().clone();
        apply(&host, &packages, &selection, &observed).unwrap();
        assert_eq!(*host.uninstalls.borrow(), vec![rg.id]);
    }

    #[test]
    fn observed_optional_catalog_survives_reload() {
        let mut l = live_list();
        let node_id = PkgId::new(Kind::Formula, "node", None);
        l.observed.insert(node_id.clone());
        l.reload();
        assert!(l.packages.iter().any(|p| p.name == "node"));
        assert!(l.selection.contains_key(&node_id));
    }

    #[test]
    fn toggle_show_all_installed_adds_unknown_formula() {
        let mut l = live_list();
        let unknown = PkgId::new(Kind::Formula, "macstrap-unknown-formula", None);
        l.observed.insert(unknown.clone());
        assert!(!l
            .packages
            .iter()
            .any(|p| p.name == "macstrap-unknown-formula"));
        l.toggle_show_all_installed();
        assert!(l.show_all_installed);
        assert!(l
            .packages
            .iter()
            .any(|p| p.name == "macstrap-unknown-formula"));
        l.toggle_show_all_installed();
        assert!(!l.show_all_installed);
        assert!(!l
            .packages
            .iter()
            .any(|p| p.name == "macstrap-unknown-formula"));
    }

    #[test]
    fn toggle_wont_uncheck_cli_essential() {
        let mut l = live_list();
        let git = PkgId::new(Kind::Formula, "git", None);
        l.selection.insert(git.clone(), true);
        l.cursor = l
            .visible()
            .into_iter()
            .position(|i| l.packages[i].id == git)
            .unwrap();
        l.toggle();
        assert!(l.selection[&git]);
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
    fn package_row_style_colors_by_install_state() {
        let mut l = live_list();
        let git = PkgId::new(Kind::Formula, "git", None);
        let node = PkgId::new(Kind::Formula, "node", None);
        l.observed.insert(git.clone());
        l.observed.insert(node.clone());
        l.reload();
        assert_eq!(
            package_row_style(l.packages.iter().find(|p| p.id == git).unwrap(), &l,).fg,
            Some(Color::Green)
        );
        assert_eq!(
            package_row_style(l.packages.iter().find(|p| p.id == node).unwrap(), &l,).fg,
            Some(Color::DarkGray)
        );
        assert_eq!(
            package_row_style(l.packages.iter().find(|p| p.name == "jq").unwrap(), &l,).fg,
            None
        );
    }

    fn long_pick_list(n: usize, cursor: usize) -> CatalogList {
        let packages: Vec<Package> = (0..n)
            .map(|i| {
                let name = format!("tool-{i:02}");
                Package {
                    id: PkgId::new(Kind::Formula, &name, None),
                    kind: Kind::Formula,
                    name: name.clone(),
                    mas_id: None,
                    title: name,
                    category: "CLI".into(),
                    description: None,
                    available: None,
                    installed_version: None,
                }
            })
            .collect();
        let config_dir = catalog::default_config_dir();
        let loaded = catalog::default_loaded();
        CatalogList {
            packages,
            selection: Selection::new(),
            observed: Observed::new(),
            loaded,
            catalog_pkg_ids: HashSet::new(),
            desired: Desired::new(),
            cursor,
            catalog_cursor: 0,
            catalog_entries: catalog::all_entries(&config_dir).unwrap(),
            config_dir,
            facts: HashMap::new(),
            filter: String::new(),
            filtering: false,
            show_all_installed: false,
            list_state: ListState::default(),
            catalog_list_state: ListState::default(),
            pick_rows: 10,
            catalog_rows: 10,
        }
    }

    fn buffer_text(terminal: &ratatui::Terminal<ratatui::backend::TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect()
    }

    #[test]
    fn pick_list_scrolls_to_cursor() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut list = long_pick_list(30, 25);
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_pick(frame, &mut list)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("tool-25"), "selected row should be visible");
        assert!(
            text.contains("tool-17"),
            "viewport should show a page of rows"
        );
        assert!(
            !text.contains("tool-00"),
            "first row should scroll off screen"
        );
    }

    #[test]
    fn pick_list_page_down_scrolls_viewport() {
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        let mut list = long_pick_list(80, 0);
        let backend = TestBackend::new(80, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw_pick(frame, &mut list)).unwrap();
        list.scroll_pick(list.pick_rows as isize);
        terminal.draw(|frame| draw_pick(frame, &mut list)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            !text.contains("tool-00"),
            "page down should scroll the first row off a tall terminal"
        );
        assert!(text.contains(&format!("tool-{:02}", list.cursor)));
    }

    #[test]
    fn page_keys_move_cursor() {
        let mut l = long_pick_list(30, 0);
        l.pick_rows = 10;
        assert!(matches!(
            l.handle_key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE)),
            PickAction::Continue
        ));
        assert_eq!(l.cursor, 10);
        assert!(matches!(
            l.handle_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
            PickAction::Continue
        ));
        assert_eq!(l.cursor, 29);
        assert!(matches!(
            l.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
            PickAction::Continue
        ));
        assert_eq!(l.cursor, 0);
        l.cursor = 15;
        assert!(matches!(
            l.handle_key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE)),
            PickAction::Continue
        ));
        assert_eq!(l.cursor, 5);
    }

    #[test]
    fn repeat_and_release_keys() {
        let press = KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE);
        let mut repeat = press;
        repeat.kind = KeyEventKind::Repeat;
        let mut release = press;
        release.kind = KeyEventKind::Release;
        assert!(actionable_key(&press));
        assert!(actionable_key(&repeat));
        assert!(!actionable_key(&release));
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

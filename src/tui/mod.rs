//! Interactive metadata workstation (ratatui + CYBERGRID theme).
//!
//! Open a photo to browse its tags beside a privacy panel that shows what
//! the file gives away; open a folder to see every photo's risk and scrub
//! them all at once. Changes stay in memory until you save.

use crate::meta::audit::human_bytes;
use crate::meta::scrub::{self, parse_shift};
use crate::meta::{
    collect_images, list_presets, resolve_output, write_atomic, Audit, Keep, MetaDoc, Outcome,
    Preset, Severity, TagView,
};
use crate::theme::Theme;
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Mode {
    Browse,
    Filter,
    Edit,
    PathInput,
    ShiftInput,
    /// Preview of a full scrub, waiting for y/n.
    ConfirmScrub,
    PresetPicker,
    /// Before/after of the last operation.
    Report,
    Message,
    Folder,
    ConfirmBatch,
}

pub fn run(initial: Option<PathBuf>) -> Result<()> {
    let mut terminal = setup()?;
    let result = App::new(initial).and_then(|mut app| app.run(&mut terminal));
    restore(&mut terminal)?;
    result
}

fn setup() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode().context("enable raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).context("create terminal")
}

fn restore(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode().ok();
    execute!(terminal.backend_mut(), LeaveAlternateScreen).ok();
    terminal.show_cursor().ok();
    Ok(())
}

struct FolderEntry {
    path: PathBuf,
    audit: std::result::Result<Audit, String>,
}

struct Folder {
    dir: PathBuf,
    entries: Vec<FolderEntry>,
    state: ListState,
}

struct App {
    theme: Theme,
    mode: Mode,
    doc: Option<MetaDoc>,
    audit: Option<Audit>,
    tags: Vec<TagView>,
    filtered: Vec<usize>,
    list_state: ListState,
    filter: String,
    input: String,
    message: String,
    presets: Vec<Preset>,
    preset_idx: usize,
    keep: Keep,
    /// A dry-run scrub waiting for confirmation.
    preview: Option<Outcome>,
    /// The last applied operation, for the report.
    report: Option<(String, Outcome)>,
    folder: Option<Folder>,
}

impl App {
    fn new(initial: Option<PathBuf>) -> Result<Self> {
        let theme = Theme::from_cybercore();
        let mut app = Self {
            theme,
            mode: Mode::Browse,
            doc: None,
            audit: None,
            tags: Vec::new(),
            filtered: Vec::new(),
            list_state: ListState::default(),
            filter: String::new(),
            input: String::new(),
            message: String::new(),
            presets: list_presets().to_vec(),
            preset_idx: 0,
            keep: Keep::default(),
            preview: None,
            report: None,
            folder: None,
        };
        match initial {
            Some(path) => app.open_path(&path)?,
            None => {
                app.mode = Mode::PathInput;
                app.message = "enter an image or folder path".into();
            }
        }
        Ok(app)
    }

    fn open_path(&mut self, path: &Path) -> Result<()> {
        if path.is_dir() {
            return self.open_folder(path);
        }
        let doc = MetaDoc::open(path)?;
        self.reload_from(doc);
        self.mode = Mode::Browse;
        self.message = format!("opened {}", path.display());
        Ok(())
    }

    fn open_folder(&mut self, dir: &Path) -> Result<()> {
        let files = collect_images(&[dir.to_path_buf()], false)?;
        let entries = files
            .into_iter()
            .map(|path| {
                let audit = std::fs::read(&path)
                    .map(|b| crate::meta::audit::audit(&b))
                    .map_err(|e| e.to_string());
                FolderEntry { path, audit }
            })
            .collect::<Vec<_>>();
        let mut state = ListState::default();
        if !entries.is_empty() {
            state.select(Some(0));
        }
        let high = entries
            .iter()
            .filter(|e| matches!(&e.audit, Ok(a) if a.risk == Some(Severity::High)))
            .count();
        self.message = format!(
            "{} photo(s) in {}, {high} high risk",
            entries.len(),
            dir.display()
        );
        self.folder = Some(Folder {
            dir: dir.to_path_buf(),
            entries,
            state,
        });
        self.doc = None;
        self.mode = Mode::Folder;
        Ok(())
    }

    fn reload_from(&mut self, doc: MetaDoc) {
        self.audit = Some(doc.audit());
        self.tags = doc.tags();
        self.doc = Some(doc);
        self.refilter();
    }

    fn refresh(&mut self) {
        if let Some(doc) = &self.doc {
            self.audit = Some(doc.audit());
            self.tags = doc.tags();
            self.refilter();
        }
    }

    fn refilter(&mut self) {
        let q = self.filter.to_ascii_lowercase();
        self.filtered = self
            .tags
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                q.is_empty()
                    || t.name.to_ascii_lowercase().contains(&q)
                    || t.value.to_ascii_lowercase().contains(&q)
                    || t.group.as_str().to_ascii_lowercase().contains(&q)
            })
            .map(|(i, _)| i)
            .collect();
        if self.filtered.is_empty() {
            self.list_state.select(None);
        } else {
            let idx = self.list_state.selected().unwrap_or(0);
            let idx = idx.min(self.filtered.len() - 1);
            self.list_state.select(Some(idx));
        }
    }

    fn selected_tag(&self) -> Option<&TagView> {
        let idx = self.list_state.selected()?;
        let tag_idx = *self.filtered.get(idx)?;
        self.tags.get(tag_idx)
    }

    fn run(&mut self, terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
        loop {
            terminal.draw(|f| self.draw(f))?;
            if !event::poll(Duration::from_millis(200))? {
                continue;
            }
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                break;
            }
            if self.handle_key(key.code)? {
                break;
            }
        }
        Ok(())
    }

    /// Returns true when the app should quit.
    fn handle_key(&mut self, code: KeyCode) -> Result<bool> {
        match self.mode {
            Mode::PathInput => self.handle_path_input(code),
            Mode::Filter => self.handle_filter(code),
            Mode::Edit => self.handle_edit(code),
            Mode::ShiftInput => self.handle_shift(code),
            Mode::ConfirmScrub => self.handle_confirm_scrub(code),
            Mode::PresetPicker => self.handle_presets(code),
            Mode::Folder => self.handle_folder(code),
            Mode::ConfirmBatch => self.handle_confirm_batch(code),
            Mode::Report | Mode::Message => {
                self.mode = if self.doc.is_some() {
                    Mode::Browse
                } else if self.folder.is_some() {
                    Mode::Folder
                } else {
                    Mode::PathInput
                };
                Ok(false)
            }
            Mode::Browse => self.handle_browse(code),
        }
    }

    fn show_report(&mut self, label: &str, result: Result<Outcome>) {
        match result {
            Ok(outcome) => {
                self.refresh();
                self.message = format!(
                    "{label}: cleared {} finding(s) (unsaved)",
                    outcome.cleared()
                );
                self.report = Some((label.to_string(), outcome));
                self.mode = Mode::Report;
            }
            Err(err) => {
                self.message = format!("{label} failed: {err:#}");
                self.mode = Mode::Message;
            }
        }
    }

    fn handle_browse(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Esc | KeyCode::Char('b') if self.folder.is_some() => {
                self.doc = None;
                self.mode = Mode::Folder;
                self.message = "back to folder".into();
            }
            KeyCode::Esc => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down => self.move_sel(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_sel(-1),
            KeyCode::Char('/') => {
                self.mode = Mode::Filter;
                self.message = "filter".into();
            }
            KeyCode::Char('o') => {
                self.mode = Mode::PathInput;
                self.input.clear();
                self.message = "open image or folder".into();
            }
            KeyCode::Enter => {
                let editable = self
                    .selected_tag()
                    .map(|t| (t.name.clone(), t.value.clone()));
                if let Some((name, value)) = editable {
                    if crate::meta::tags::string_tag_by_name(&name, "").is_ok() {
                        self.input = value;
                        self.mode = Mode::Edit;
                        self.message = format!("edit {name}");
                    } else {
                        self.message = format!("{name} is not a string-editable tag");
                    }
                }
            }
            KeyCode::Char('g') => {
                if let Some(doc) = self.doc.as_mut() {
                    let r = doc.strip_gps();
                    self.show_report("remove location", r);
                }
            }
            KeyCode::Char('X') => {
                if let Some(doc) = &self.doc {
                    match scrub::scrub(&doc.bytes, &self.keep) {
                        Ok(preview) => {
                            self.preview = Some(preview);
                            self.mode = Mode::ConfirmScrub;
                            self.message = "scrub everything? y apply · n cancel".into();
                        }
                        Err(err) => {
                            self.message = format!("can't scrub: {err:#}");
                            self.mode = Mode::Message;
                        }
                    }
                }
            }
            KeyCode::Char('1') => {
                self.keep.icc = !self.keep.icc;
                self.message = format!("scrub keeps: {}", self.keep.describe());
            }
            KeyCode::Char('2') => {
                self.keep.orientation = !self.keep.orientation;
                self.message = format!("scrub keeps: {}", self.keep.describe());
            }
            KeyCode::Char('3') => {
                self.keep.date = !self.keep.date;
                self.message = format!("scrub keeps: {}", self.keep.describe());
            }
            KeyCode::Char('t') => {
                self.input.clear();
                self.mode = Mode::ShiftInput;
                self.message = "shift every timestamp by, e.g. +3d, -2h30m".into();
            }
            KeyCode::Char('p') => {
                self.mode = Mode::PresetPicker;
                self.preset_idx = 0;
                self.message = "presets — Enter apply, Esc cancel".into();
            }
            KeyCode::Char('r') => {
                if self.report.is_some() {
                    self.mode = Mode::Report;
                }
            }
            KeyCode::Char('s') => self.save(false)?,
            KeyCode::Char('S') => self.save(true)?,
            KeyCode::Char('e') => self.export_sidecar()?,
            _ => {}
        }
        Ok(false)
    }

    fn handle_folder(&mut self, code: KeyCode) -> Result<bool> {
        let Some(folder) = self.folder.as_mut() else {
            self.mode = Mode::PathInput;
            return Ok(false);
        };
        let n = folder.entries.len();
        match code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('j') | KeyCode::Down if n > 0 => {
                let i = folder.state.selected().unwrap_or(0);
                folder.state.select(Some((i + 1).min(n - 1)));
            }
            KeyCode::Char('k') | KeyCode::Up if n > 0 => {
                let i = folder.state.selected().unwrap_or(0);
                folder.state.select(Some(i.saturating_sub(1)));
            }
            KeyCode::Enter => {
                let path = folder
                    .state
                    .selected()
                    .and_then(|i| folder.entries.get(i))
                    .map(|e| e.path.clone());
                if let Some(path) = path {
                    match MetaDoc::open(&path) {
                        Ok(doc) => {
                            self.reload_from(doc);
                            self.mode = Mode::Browse;
                            self.message = format!("opened {} · b back to folder", path.display());
                        }
                        Err(err) => {
                            self.message = format!("open failed: {err:#}");
                            self.mode = Mode::Message;
                        }
                    }
                }
            }
            KeyCode::Char('A') if n > 0 => {
                self.mode = Mode::ConfirmBatch;
                self.message = format!(
                    "scrub all {n} photo(s) to *.cybermeta.* copies, keeping {}? y/n",
                    self.keep.describe()
                );
            }
            KeyCode::Char('R') => {
                let dir = folder.dir.clone();
                self.open_folder(&dir)?;
            }
            KeyCode::Char('o') => {
                self.mode = Mode::PathInput;
                self.input.clear();
                self.message = "open image or folder".into();
            }
            KeyCode::Char('1') => self.keep.icc = !self.keep.icc,
            KeyCode::Char('2') => self.keep.orientation = !self.keep.orientation,
            KeyCode::Char('3') => self.keep.date = !self.keep.date,
            _ => {}
        }
        Ok(false)
    }

    fn handle_confirm_batch(&mut self, code: KeyCode) -> Result<bool> {
        if !matches!(code, KeyCode::Char('y') | KeyCode::Char('Y')) {
            self.mode = Mode::Folder;
            self.message = "batch scrub cancelled".into();
            return Ok(false);
        }
        let Some(folder) = &self.folder else {
            return Ok(false);
        };
        let (mut ok, mut failed, mut first_err) = (0, 0, None);
        for e in &folder.entries {
            let r = (|| -> Result<()> {
                let bytes = std::fs::read(&e.path)?;
                let out = scrub::scrub(&bytes, &self.keep)?;
                write_atomic(&resolve_output(&e.path, None, false)?, &out.bytes)
            })();
            match r {
                Ok(()) => ok += 1,
                Err(err) => {
                    failed += 1;
                    first_err.get_or_insert_with(|| format!("{}: {err:#}", e.path.display()));
                }
            }
        }
        let dir = folder.dir.clone();
        self.open_folder(&dir)?;
        self.message = match first_err {
            None => format!("scrubbed {ok} photo(s) → *.cybermeta.* copies"),
            Some(e) => format!("scrubbed {ok}, {failed} failed — {e}"),
        };
        Ok(false)
    }

    fn handle_path_input(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Esc => {
                if self.doc.is_some() {
                    self.mode = Mode::Browse;
                } else if self.folder.is_some() {
                    self.mode = Mode::Folder;
                } else {
                    return Ok(true);
                }
            }
            KeyCode::Enter => {
                let path = PathBuf::from(self.input.trim());
                match self.open_path(&path) {
                    Ok(()) => self.input.clear(),
                    Err(err) => {
                        self.message = format!("open failed: {err:#}");
                        self.mode = Mode::Message;
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        Ok(false)
    }

    fn handle_filter(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Esc => {
                self.filter.clear();
                self.refilter();
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                self.mode = Mode::Browse;
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.refilter();
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.refilter();
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_edit(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.input.clear();
            }
            KeyCode::Enter => {
                if let Some(tag) = self.selected_tag().cloned() {
                    if let Some(doc) = self.doc.as_mut() {
                        match doc.set_string_tag(&tag.name, &self.input) {
                            Ok(()) => {
                                self.refresh();
                                self.message = format!("set {} (unsaved)", tag.name);
                            }
                            Err(err) => self.message = format!("edit failed: {err:#}"),
                        }
                    }
                }
                self.mode = Mode::Browse;
                self.input.clear();
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        Ok(false)
    }

    fn handle_shift(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                self.input.clear();
            }
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.input);
                match parse_shift(&text) {
                    Ok(seconds) => {
                        if let Some(doc) = self.doc.as_mut() {
                            let r = doc.date_shift(seconds);
                            self.show_report("date shift", r);
                        }
                    }
                    Err(err) => {
                        self.message = format!("{err:#}");
                        self.mode = Mode::Message;
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c) => self.input.push(c),
            _ => {}
        }
        Ok(false)
    }

    fn handle_confirm_scrub(&mut self, code: KeyCode) -> Result<bool> {
        let preview = self.preview.take();
        match (code, preview) {
            (KeyCode::Char('y') | KeyCode::Char('Y'), Some(_)) => {
                if let Some(doc) = self.doc.as_mut() {
                    let r = doc.scrub(&self.keep);
                    self.show_report("scrub", r);
                }
            }
            _ => {
                self.mode = Mode::Browse;
                self.message = "scrub cancelled".into();
            }
        }
        Ok(false)
    }

    fn handle_presets(&mut self, code: KeyCode) -> Result<bool> {
        match code {
            KeyCode::Esc => {
                self.mode = Mode::Browse;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.preset_idx + 1 < self.presets.len() {
                    self.preset_idx += 1;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.preset_idx = self.preset_idx.saturating_sub(1);
            }
            KeyCode::Enter => {
                let preset = self.presets[self.preset_idx];
                if let Some(doc) = self.doc.as_mut() {
                    let r = doc.apply_preset(preset);
                    self.show_report(preset.as_str(), r);
                } else {
                    self.mode = Mode::Browse;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn move_sel(&mut self, delta: i32) {
        if self.filtered.is_empty() {
            return;
        }
        let cur = self.list_state.selected().unwrap_or(0) as i32;
        let next = (cur + delta).clamp(0, (self.filtered.len() as i32) - 1) as usize;
        self.list_state.select(Some(next));
    }

    fn save(&mut self, in_place: bool) -> Result<()> {
        let Some(doc) = self.doc.as_mut() else {
            self.message = "no file open".into();
            return Ok(());
        };
        match doc.save(None, in_place) {
            Ok(path) => {
                self.message = if in_place {
                    format!("saved in-place {}", path.display())
                } else {
                    format!("saved {}", path.display())
                };
                self.refresh();
            }
            Err(err) => self.message = format!("save failed: {err:#}"),
        }
        Ok(())
    }

    fn export_sidecar(&mut self) -> Result<()> {
        let Some(doc) = &self.doc else {
            self.message = "no file open".into();
            return Ok(());
        };
        let json = doc.export_string(crate::meta::ExportFormat::Json)?;
        let out = doc.path.with_extension("cybermeta.json");
        std::fs::write(&out, json)?;
        self.message = format!("exported {}", out.display());
        Ok(())
    }

    // ------------------------------------------------------------ drawing --

    fn severity_color(&self, s: Option<Severity>) -> Color {
        match s {
            None => self.theme.acid_green,
            Some(Severity::Low) => self.theme.cyan,
            Some(Severity::Medium) => self.theme.orange,
            Some(Severity::High) => self.theme.red,
        }
    }

    fn risk_label(s: Option<Severity>) -> &'static str {
        match s {
            None => "CLEAN",
            Some(Severity::Low) => "LOW",
            Some(Severity::Medium) => "MEDIUM",
            Some(Severity::High) => "HIGH",
        }
    }

    fn panel(&self, title: impl Into<String>) -> Block<'static> {
        Block::default()
            .title(title.into())
            .borders(Borders::ALL)
            .border_style(Style::default().fg(self.theme.line))
            .style(Style::default().bg(self.theme.panel))
    }

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(5),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(area);

        self.draw_header(frame, chunks[0]);
        if self.doc.is_none() && self.folder.is_some() {
            self.draw_folder(frame, chunks[1]);
        } else {
            self.draw_body(frame, chunks[1]);
        }
        self.draw_input(frame, chunks[2]);
        self.draw_status(frame, chunks[3]);

        match self.mode {
            Mode::PresetPicker => self.draw_preset_modal(frame, area),
            Mode::ConfirmScrub => {
                if let Some(p) = &self.preview {
                    let lines = self.outcome_lines(p, true);
                    self.draw_modal(
                        frame,
                        area,
                        " scrub preview — y apply · n cancel ",
                        lines,
                        self.theme.hot_pink,
                    );
                }
            }
            Mode::Report => {
                if let Some((label, o)) = &self.report {
                    let lines = self.outcome_lines(o, false);
                    self.draw_modal(
                        frame,
                        area,
                        format!(" {label}: before → after "),
                        lines,
                        self.theme.acid_green,
                    );
                }
            }
            _ => {}
        }
    }

    fn draw_header(&self, frame: &mut ratatui::Frame, area: Rect) {
        let dirty = self
            .doc
            .as_ref()
            .map(|d| if d.dirty { " ● unsaved" } else { "" })
            .unwrap_or("");
        let path = match (&self.doc, &self.folder) {
            (Some(d), _) => d.path.display().to_string(),
            (None, Some(f)) => format!("{}/", f.dir.display()),
            _ => "(no file)".into(),
        };
        let risk = self
            .audit
            .as_ref()
            .filter(|_| self.doc.is_some())
            .map(|a| a.risk);
        let mut spans = vec![
            Span::styled(
                " cybermeta ",
                Style::default()
                    .fg(self.theme.bg)
                    .bg(self.theme.acid_green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {path}{dirty} "),
                Style::default().fg(self.theme.white),
            ),
        ];
        if let Some(r) = risk {
            spans.push(Span::styled(
                format!(" {} ", Self::risk_label(r)),
                Style::default()
                    .fg(self.theme.bg)
                    .bg(self.severity_color(r))
                    .add_modifier(Modifier::BOLD),
            ));
        }
        spans.push(Span::styled(
            format!(" theme:{} ", self.theme.name),
            Style::default().fg(self.theme.muted),
        ));
        frame.render_widget(
            Paragraph::new(Line::from(spans)).block(self.panel("")),
            area,
        );
    }

    fn draw_body(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let right = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(6), Constraint::Length(8)])
            .split(panes[1]);

        let items: Vec<ListItem> = self
            .filtered
            .iter()
            .map(|&i| {
                let t = &self.tags[i];
                let label = format!(
                    "[{}] {:<22} {}",
                    t.group.as_str(),
                    t.name,
                    truncate(&t.value, 40)
                );
                ListItem::new(Span::styled(label, Style::default().fg(self.theme.white)))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .title(format!(" EXIF tags ({}) ", self.tags.len()))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(self.theme.line))
                    .style(Style::default().bg(self.theme.bg)),
            )
            .highlight_style(
                Style::default()
                    .bg(self.theme.line)
                    .fg(self.theme.acid_green)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, panes[0], &mut self.list_state);

        if let Some(a) = &self.audit {
            self.draw_privacy(frame, right[0], a, true);
        }

        let detail = if let Some(tag) = self.selected_tag() {
            format!(
                "name:  {}\nhex:   0x{:04X}\ngroup: {}\nvalue: {}",
                tag.name,
                tag.hex,
                tag.group.as_str(),
                tag.value
            )
        } else {
            "no tag selected".into()
        };
        let detail = Paragraph::new(detail)
            .wrap(Wrap { trim: false })
            .style(Style::default().fg(self.theme.cyan))
            .block(self.panel(" detail "));
        frame.render_widget(detail, right[1]);
    }

    fn draw_privacy(&self, frame: &mut ratatui::Frame, area: Rect, a: &Audit, keep_line: bool) {
        let color = self.severity_color(a.risk);
        let mut lines: Vec<Line> = Vec::new();
        if a.findings.is_empty() {
            lines.push(Line::from(Span::styled(
                "✔ nothing found — safe to share",
                Style::default()
                    .fg(self.theme.acid_green)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        for f in &a.findings {
            let c = self.severity_color(Some(f.severity));
            lines.push(Line::from(vec![
                Span::styled("● ", Style::default().fg(c)),
                Span::styled(
                    format!("{:<9}", f.category.as_str()),
                    Style::default().fg(c),
                ),
                Span::styled(
                    format!("{} ", f.label),
                    Style::default().fg(self.theme.white),
                ),
                Span::styled(
                    truncate(&f.value, 48),
                    Style::default().fg(self.theme.muted),
                ),
                Span::styled(
                    format!("  {}", f.source),
                    Style::default().fg(self.theme.purple),
                ),
            ]));
        }
        if !a.scrubbable {
            lines.push(Line::from(Span::styled(
                format!("! {} can't be scrubbed losslessly yet", a.format.as_str()),
                Style::default().fg(self.theme.orange),
            )));
        }
        if let Some(g) = &a.gps {
            lines.push(Line::from(Span::styled(
                format!("map: {}", g.map_url),
                Style::default().fg(self.theme.muted),
            )));
        }
        if keep_line {
            let mark = |on: bool| if on { "✔" } else { "✘" };
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "scrub keeps: 1 colour {} · 2 rotation {} · 3 date {}",
                    mark(self.keep.icc),
                    mark(self.keep.orientation),
                    mark(self.keep.date)
                ),
                Style::default().fg(self.theme.hot_pink),
            )));
        }
        let title = format!(
            " privacy · {} · {} high {} medium {} low ",
            Self::risk_label(a.risk),
            a.count(Severity::High),
            a.count(Severity::Medium),
            a.count(Severity::Low)
        );
        let widget = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(Span::styled(
                    title,
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color))
                .style(Style::default().bg(self.theme.panel)),
        );
        frame.render_widget(widget, area);
    }

    fn draw_folder(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let panes = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        let theme = &self.theme;
        let items: Vec<ListItem> = folder
            .entries
            .iter()
            .map(|e| {
                let name = e.path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
                let (badge, color, count) = match &e.audit {
                    Ok(a) => (
                        Self::risk_label(a.risk),
                        match a.risk {
                            None => theme.acid_green,
                            Some(Severity::Low) => theme.cyan,
                            Some(Severity::Medium) => theme.orange,
                            Some(Severity::High) => theme.red,
                        },
                        format!("{} finding(s)", a.findings.len()),
                    ),
                    Err(_) => ("ERROR", theme.red, String::new()),
                };
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{badge:<7}"),
                        Style::default().fg(color).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{} ", truncate(name, 34)),
                        Style::default().fg(theme.white),
                    ),
                    Span::styled(count, Style::default().fg(theme.muted)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(
                Block::default()
                    .title(format!(" photos ({}) ", folder.entries.len()))
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(theme.line))
                    .style(Style::default().bg(theme.bg)),
            )
            .highlight_style(Style::default().bg(theme.line).add_modifier(Modifier::BOLD))
            .highlight_symbol("▶ ");
        frame.render_stateful_widget(list, panes[0], &mut folder.state);
        let selected = folder.state.selected().and_then(|i| folder.entries.get(i));
        match selected.map(|e| &e.audit) {
            Some(Ok(a)) => {
                let a = a.clone();
                self.draw_privacy(frame, panes[1], &a, true);
            }
            Some(Err(e)) => {
                let w = Paragraph::new(e.clone()).block(self.panel(" error "));
                frame.render_widget(w, panes[1]);
            }
            None => {
                let w = Paragraph::new("no photos here").block(self.panel(" privacy "));
                frame.render_widget(w, panes[1]);
            }
        }
    }

    fn outcome_lines(&self, o: &Outcome, preview: bool) -> Vec<Line<'static>> {
        let t = &self.theme;
        let mut lines = vec![Line::from(vec![
            Span::styled("before ", Style::default().fg(t.muted)),
            Span::styled(
                format!(
                    "{} ({} findings)",
                    Self::risk_label(o.before.risk),
                    o.before.findings.len()
                ),
                Style::default()
                    .fg(self.severity_color(o.before.risk))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  →  after ", Style::default().fg(t.muted)),
            Span::styled(
                format!(
                    "{} ({} findings)",
                    Self::risk_label(o.after.risk),
                    o.after.findings.len()
                ),
                Style::default()
                    .fg(self.severity_color(o.after.risk))
                    .add_modifier(Modifier::BOLD),
            ),
        ])];
        lines.push(Line::from(""));
        let verb = if preview { "will remove" } else { "removed" };
        if o.removed.is_empty() {
            lines.push(Line::from(Span::styled(
                format!("{verb}: no blocks"),
                Style::default().fg(t.muted),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                format!("{verb}:"),
                Style::default().fg(t.white),
            )));
        }
        for r in &o.removed {
            lines.push(Line::from(vec![
                Span::styled("  − ", Style::default().fg(t.red)),
                Span::styled(
                    format!("{:<22}", r.kind_label()),
                    Style::default().fg(t.white),
                ),
                Span::styled(
                    format!("{} · {}", r.label, human_bytes(r.bytes)),
                    Style::default().fg(t.muted),
                ),
            ]));
        }
        let gone: Vec<_> = o
            .before
            .findings
            .iter()
            .filter(|f| {
                !o.after
                    .findings
                    .iter()
                    .any(|g| g.label == f.label && g.source == f.source && g.value == f.value)
            })
            .collect();
        if !gone.is_empty() {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                if preview {
                    "no longer readable:"
                } else {
                    "gone:"
                },
                Style::default().fg(t.white),
            )));
            for f in gone.iter().take(12) {
                lines.push(Line::from(vec![
                    Span::styled("  ✔ ", Style::default().fg(t.acid_green)),
                    Span::styled(format!("{} ", f.label), Style::default().fg(t.white)),
                    Span::styled(truncate(&f.value, 40), Style::default().fg(t.muted)),
                ]));
            }
            if gone.len() > 12 {
                lines.push(Line::from(Span::styled(
                    format!("  … and {} more", gone.len() - 12),
                    Style::default().fg(t.muted),
                )));
            }
        }
        for n in &o.notes {
            lines.push(Line::from(Span::styled(
                format!("note: {n}"),
                Style::default().fg(t.orange),
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            if preview {
                format!("keeps: {} (toggle with 1/2/3 first)", self.keep.describe())
            } else {
                "unsaved · s save a copy · S overwrite · any key closes".to_string()
            },
            Style::default().fg(t.hot_pink),
        )));
        lines
    }

    fn draw_modal(
        &self,
        frame: &mut ratatui::Frame,
        area: Rect,
        title: impl Into<String>,
        lines: Vec<Line<'static>>,
        border: Color,
    ) {
        let width = 84u16.min(area.width.saturating_sub(4));
        let height = (lines.len() as u16 + 2).min(area.height.saturating_sub(2));
        let rect = Rect::new(
            (area.width.saturating_sub(width)) / 2,
            (area.height.saturating_sub(height)) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, rect);
        let widget = Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .title(title.into())
                .borders(Borders::ALL)
                .border_style(Style::default().fg(border))
                .style(Style::default().bg(self.theme.panel)),
        );
        frame.render_widget(widget, rect);
    }

    fn draw_input(&self, frame: &mut ratatui::Frame, area: Rect) {
        let (title, text) = match self.mode {
            Mode::Filter => (" filter ", self.filter.as_str()),
            Mode::Edit => (" edit value ", self.input.as_str()),
            Mode::PathInput => (" path ", self.input.as_str()),
            Mode::ShiftInput => (" shift timestamps by ", self.input.as_str()),
            Mode::ConfirmScrub => (" confirm ", "scrub everything? y/n"),
            Mode::ConfirmBatch => (" confirm ", "scrub every photo in this folder? y/n"),
            Mode::PresetPicker => (" presets ", "j/k select · Enter apply"),
            Mode::Folder | Mode::Message | Mode::Report if self.doc.is_none() => (
                " folder ",
                "j/k move · Enter open · A scrub all · 1/2/3 keep · R rescan · o open · q quit",
            ),
            _ => (
                " actions ",
                "X scrub · g location · p preset · t shift dates · 1/2/3 keep · Enter edit · / filter · s save · S in-place · e export · r report · o open · q quit",
            ),
        };
        let widget = Paragraph::new(Span::styled(
            text.to_string(),
            Style::default().fg(self.theme.hot_pink),
        ))
        .block(self.panel(title));
        frame.render_widget(widget, area);
    }

    fn draw_status(&self, frame: &mut ratatui::Frame, area: Rect) {
        let msg = Paragraph::new(Span::styled(
            format!(" {}", self.message),
            Style::default().fg(self.theme.muted),
        ))
        .style(Style::default().bg(self.theme.bg));
        frame.render_widget(msg, area);
    }

    fn draw_preset_modal(&self, frame: &mut ratatui::Frame, area: Rect) {
        let lines: Vec<Line<'static>> = self
            .presets
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let marker = if i == self.preset_idx { "▶" } else { " " };
                let style = if i == self.preset_idx {
                    Style::default()
                        .fg(self.theme.acid_green)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(self.theme.white)
                };
                Line::from(Span::styled(
                    format!("{marker} {:<17} {}", p.as_str(), p.description()),
                    style,
                ))
            })
            .collect();
        self.draw_modal(
            frame,
            area,
            " scrub / spoof presets ",
            lines,
            self.theme.hot_pink,
        );
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

use crate::scanner::{Disk, Found, age, human};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};
use crate::tree::Tree;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui::{DefaultTerminal, Frame};
use std::io;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const BY_TYPE: usize = 0;
const BY_DIR: usize = 1;

const META_WIDTH: usize = 14;
const SIZE_WIDTH: usize = 10;
const AGE_WIDTH: usize = 6;
const BAR_WIDTH: usize = 8;

const KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("enter", "open"),
    ("<", "back"),
    ("x", "select"),
    ("a", "all"),
    ("e", "expand"),
    ("s", "sort"),
    ("tab", "view"),
    ("d", "delete"),
    ("q", "quit"),
];

struct App<'a> {
    items: &'a [Found],
    root: &'a str,
    disk: Option<Disk>,
    dry: bool,
    /// Deletes one path; a no-op in a dry run.
    remove: &'a dyn Fn(&Path) -> io::Result<()>,
    /// Re-reads free space after a deletion.
    disk_now: &'a dyn Fn() -> Option<Disk>,
    /// Selection per item, shared by both views.
    picked: Vec<bool>,
    /// Items deleted so far this session.
    gone: Vec<bool>,
    session: Session,
    trees: [Tree; 2],
    view: usize,
    total: u64,
    /// Rows visible in the list, as of the last draw.
    height: usize,
    /// Items the open confirmation dialog is asking about; empty when closed.
    pending: Vec<usize>,
    /// Whether "Yes" has focus in the confirmation dialog.
    confirm_yes: bool,
    /// Stalest first rather than biggest first.
    by_age: bool,
    notice: Option<String>,
}

enum Action {
    Quit,
    Delete,
}

/// What a run of the picker did.
#[derive(Default)]
pub struct Session {
    pub deleted: Vec<usize>,
    pub failures: Vec<(usize, io::Error)>,
}

/// Runs the interactive picker, deleting in place as the user confirms, and
/// returns what was deleted by index into `items`.
pub fn run(
    items: &[Found],
    root: &str,
    disk: Option<Disk>,
    dry: bool,
    remove: &dyn Fn(&Path) -> io::Result<()>,
    disk_now: &dyn Fn() -> Option<Disk>,
) -> io::Result<Session> {
    let mut app = App {
        items,
        root,
        disk,
        dry,
        remove,
        disk_now,
        picked: vec![false; items.len()],
        gone: vec![false; items.len()],
        session: Session::default(),
        trees: [Tree::by_type(items), Tree::by_dir(items)],
        view: BY_TYPE,
        total: items.iter().map(|f| f.size).sum(),
        height: 1,
        pending: Vec::new(),
        confirm_yes: false,
        by_age: false,
        notice: None,
    };

    let mut terminal = ratatui::init();
    let result = app.event_loop(&mut terminal);
    ratatui::restore();
    result.map(|()| app.session)
}

/// Pads or truncates `text` to exactly `width` terminal columns.
fn fit(text: &str, width: usize) -> String {
    let used = text.width();
    if used <= width {
        return format!("{text}{}", " ".repeat(width - used));
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    if width > 0 {
        out.push('…');
        used += 1;
    }
    out.push_str(&" ".repeat(width - used));
    out
}

/// Recently touched things stand out as a warning; long-idle ones as fair
/// game.
fn age_style(modified: Option<SystemTime>) -> Style {
    const DAY: u64 = 24 * 3600;
    let idle = modified.and_then(|at| at.elapsed().ok()).unwrap_or(Duration::ZERO).as_secs();
    match idle {
        s if s < 7 * DAY => Style::new().yellow(),
        s if s >= 90 * DAY => Style::new().green(),
        _ => Style::new().dim(),
    }
}

fn size_color(bytes: u64) -> Color {
    const MB: u64 = 1024 * 1024;
    match bytes {
        b if b >= 1024 * MB => Color::Red,
        b if b >= 100 * MB => Color::Yellow,
        _ => Color::Reset,
    }
}

impl App<'_> {
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            let Event::Key(key) = event::read()? else { continue };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match self.on_key(key) {
                Some(Action::Quit) => return Ok(()),
                Some(Action::Delete) => {
                    self.delete_pending(terminal)?;
                    // Nothing left to pick from.
                    if self.gone.iter().all(|&gone| gone) {
                        return Ok(());
                    }
                }
                None => {}
            }
        }
    }

    /// Deletes what the confirmation dialog asked about and takes it out of
    /// the tree, leaving the picker open for the next decision.
    fn delete_pending(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        let pending = std::mem::take(&mut self.pending);
        let mut freed = 0;
        let mut count = 0;
        let mut last_error = None;
        let mut drawn: Option<Instant> = None;
        let items = self.items;

        for (n, &i) in pending.iter().enumerate() {
            let item = &items[i];
            // Redrawing per item would cost more than deleting small ones.
            if drawn.is_none_or(|at| at.elapsed() >= Duration::from_millis(50)) {
                self.notice = Some(format!("Deleting {}/{}  {}", n + 1, pending.len(), item.rel.display()));
                terminal.draw(|frame| self.draw(frame))?;
                drawn = Some(Instant::now());
            }
            self.session.failures.retain(|(failed, _)| *failed != i);
            match (self.remove)(&item.path) {
                Ok(()) => {
                    self.gone[i] = true;
                    self.picked[i] = false;
                    self.session.deleted.push(i);
                    freed += item.size;
                    count += 1;
                }
                Err(err) => {
                    last_error = Some(format!("{}: {err}", item.rel.display()));
                    self.session.failures.push((i, err));
                }
            }
        }

        self.total -= freed;
        for tree in &mut self.trees {
            tree.prune(&self.gone, self.items);
            tree.recount(&self.picked);
        }
        self.disk = match self.dry {
            true => self.disk.map(|disk| Disk { free: disk.free + freed, ..disk }),
            false => (self.disk_now)().or(self.disk),
        };
        let verb = if self.dry { "Would have freed" } else { "Freed" };
        let noun = if count == 1 { "item" } else { "items" };
        self.notice = Some(match last_error {
            Some(error) => format!("{verb} {} in {count} {noun}. Failed: {error}", human(freed)),
            None => format!("{verb} {} in {count} {noun}", human(freed)),
        });
        Ok(())
    }

    /// What `d` acts on: the selection if there is one, else the row under
    /// the cursor.
    fn deletion_targets(&self) -> Vec<usize> {
        if self.picked_count() > 0 {
            return (0..self.items.len()).filter(|&i| self.picked[i]).collect();
        }
        let tree = &self.trees[self.view];
        tree.leaves(tree.current()).into_iter().filter(|&i| !self.gone[i]).collect()
    }

    fn tree(&mut self) -> &mut Tree {
        &mut self.trees[self.view]
    }

    fn picked_count(&self) -> usize {
        self.picked.iter().filter(|&&p| p).count()
    }

    fn picked_size(&self) -> u64 {
        self.items.iter().zip(&self.picked).filter(|&(_, &p)| p).map(|(f, _)| f.size).sum()
    }

    fn set_picked(&mut self, leaves: &[usize]) {
        let on = !leaves.iter().all(|&i| self.picked[i]);
        for &i in leaves {
            self.picked[i] = on;
        }
        for tree in &mut self.trees {
            tree.recount(&self.picked);
        }
    }

    fn on_key(&mut self, key: KeyEvent) -> Option<Action> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Action::Quit);
        }
        if !self.pending.is_empty() {
            match key.code {
                KeyCode::Char('y' | 'Y') => return Some(Action::Delete),
                KeyCode::Enter if self.confirm_yes => return Some(Action::Delete),
                KeyCode::Left | KeyCode::Char('h' | '<') => self.confirm_yes = true,
                KeyCode::Right | KeyCode::Char('l' | '>') => self.confirm_yes = false,
                KeyCode::Tab | KeyCode::BackTab => self.confirm_yes = !self.confirm_yes,
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('n' | 'N' | 'q') => self.pending.clear(),
                _ => {}
            }
            return None;
        }

        self.notice = None;
        let page = self.height.saturating_sub(1).max(1) as isize;
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Some(Action::Quit),
            KeyCode::Up | KeyCode::Char('k') => self.tree().move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => self.tree().move_by(1),
            KeyCode::PageUp => self.tree().move_by(-page),
            KeyCode::PageDown => self.tree().move_by(page),
            KeyCode::Home | KeyCode::Char('g') => self.tree().cursor = 0,
            KeyCode::End | KeyCode::Char('G') => self.tree().move_to_end(),
            KeyCode::Right | KeyCode::Char('l') => self.tree().expand(),
            KeyCode::Left | KeyCode::Char('h') => self.tree().collapse(),
            KeyCode::Char('s') => {
                self.by_age = !self.by_age;
                for tree in &mut self.trees {
                    tree.sort(self.by_age);
                }
            }
            KeyCode::Char('e') => {
                let expand = !self.tree().all_expanded();
                self.tree().set_all_expanded(expand);
            }
            KeyCode::Char(' ' | 'x') => {
                let tree = &self.trees[self.view];
                let leaves = tree.leaves(tree.current());
                self.set_picked(&leaves);
            }
            KeyCode::Char('a') => {
                let all: Vec<usize> = (0..self.items.len()).filter(|&i| !self.gone[i]).collect();
                self.set_picked(&all);
            }
            KeyCode::Tab | KeyCode::BackTab => self.view = 1 - self.view,
            KeyCode::Enter | KeyCode::Char('>') => self.tree().enter(),
            KeyCode::Char('<') => self.tree().back(),
            KeyCode::Char('d') | KeyCode::Delete => {
                self.pending = self.deletion_targets();
                // Start on the harmless choice.
                self.confirm_yes = false;
            }
            _ => {}
        }
        None
    }

    fn draw(&mut self, frame: &mut Frame) {
        let [main, status, help] =
            Layout::vertical([Constraint::Min(3), Constraint::Length(1), Constraint::Length(1)])
                .areas(frame.area());

        let tab = |label: &'static str, view: usize| {
            if self.view == view {
                Span::styled(format!(" {label} "), Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD))
            } else {
                Span::styled(format!(" {label} "), Style::new().dim())
            }
        };
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().dim())
            .title(Line::from(vec![
                " safeclean ".bold(),
                if self.dry { "dry run ".yellow() } else { "".into() },
                format!("{} ", self.root).dim(),
            ]))
            .title(
                Line::from(vec![
                    format!(" sort: {} ", if self.by_age { "age" } else { "size" }).dim(),
                    tab("By type", BY_TYPE),
                    tab("By directory", BY_DIR),
                    " ".into(),
                ])
                .right_aligned(),
            );
        let inner = block.inner(main);
        frame.render_widget(block, main);

        self.height = inner.height as usize;
        let height = self.height;
        let tree = self.tree();
        if tree.cursor < tree.offset {
            tree.offset = tree.cursor;
        } else if tree.cursor >= tree.offset + height {
            tree.offset = tree.cursor + 1 - height;
        }

        let tree = &self.trees[self.view];
        let rows: Vec<Line> = tree
            .rows
            .iter()
            .enumerate()
            .skip(tree.offset)
            .take(height)
            .map(|(row, &id)| self.row(id, row == tree.cursor, inner.width as usize))
            .collect();
        frame.render_widget(Paragraph::new(rows), inner);

        frame.render_widget(self.status_line(), status);
        if let Some(disk) = self.disk {
            let picked = self.picked_size();
            let spans = if picked == 0 {
                vec![format!("{} free of {} ", human(disk.free), human(disk.total)).dim()]
            } else {
                vec![
                    format!("free {} → ", human(disk.free)).dim(),
                    human(disk.free + picked).green().bold(),
                    " ".into(),
                ]
            };
            frame.render_widget(Line::from(spans).right_aligned(), status);
        }
        let keys: Vec<Span> = KEYS
            .iter()
            .flat_map(|&(key, what)| [format!(" {key}").cyan(), format!(" {what} ").dim()])
            .collect();
        frame.render_widget(Line::from(keys), help);

        if !self.pending.is_empty() {
            self.draw_confirm(frame);
        }
    }

    /// One row, laid out so the things you decide on sit together on the
    /// left: checkbox, size, age, and only then the tree.
    fn row(&self, id: usize, is_cursor: bool, width: usize) -> Line<'static> {
        let node = &self.trees[self.view].nodes[id];
        // The cursor row is drawn reversed, where per-span colours only clash.
        let paint = |style: Style| if is_cursor { Style::new() } else { style };

        let arrow = match (node.children.is_empty(), node.expanded) {
            (true, _) => " ",
            (false, false) => "▸",
            (false, true) => "▾",
        };
        let (checkbox, checkbox_style) = match node.picked {
            0 => ("[ ]", Style::new().dim()),
            n if n == node.count => ("[x]", Style::new().green()),
            _ => ("[-]", Style::new().yellow()),
        };
        let meta = match (node.leaf, node.picked) {
            // A type header already says what its leaves are.
            (Some(_), _) if self.view == BY_TYPE => String::new(),
            (Some(_), _) => node.tag.map_or(String::new(), |t| t.name().to_string()),
            (None, 0) if node.count == 1 => "1 item".to_string(),
            (None, 0) => format!("{} items", node.count),
            (None, picked) => format!("{picked}/{} items", node.count),
        };

        let mut spans = vec![
            Span::raw(" "),
            Span::styled(checkbox, paint(checkbox_style)),
            Span::styled(
                format!("{:>SIZE_WIDTH$}", human(node.size)),
                paint(Style::new().fg(size_color(node.size))),
            ),
        ];
        let mut used = 1 + 3 + SIZE_WIDTH;
        if width >= 90 {
            let filled = if self.total == 0 {
                0
            } else {
                ((node.size as f64 / self.total as f64) * BAR_WIDTH as f64).ceil() as usize
            };
            let filled = filled.min(BAR_WIDTH);
            spans.push(Span::raw(" "));
            spans.push(Span::styled("█".repeat(filled), paint(Style::new().blue())));
            spans.push(Span::styled("░".repeat(BAR_WIDTH - filled), paint(Style::new().dim())));
            used += 1 + BAR_WIDTH;
        }
        spans.push(Span::styled(format!("{:>AGE_WIDTH$}", age(node.modified)), paint(age_style(node.modified))));
        used += AGE_WIDTH;

        let tree = format!("  {}{arrow} ", "  ".repeat(node.depth));
        let room = width.saturating_sub(used + tree.width() + META_WIDTH + 1);
        spans.push(Span::raw(tree));
        spans.push(Span::styled(
            fit(&node.label, room),
            paint(if node.leaf.is_none() { Style::new().bold() } else { Style::new() }),
        ));
        spans.push(Span::styled(format!("{meta:>META_WIDTH$} "), paint(Style::new().dim())));

        let line = Line::from(spans);
        if is_cursor { line.style(Style::new().add_modifier(Modifier::REVERSED)) } else { line }
    }

    fn status_line(&self) -> Line<'static> {
        if let Some(notice) = &self.notice {
            return Line::from(format!(" {notice}").yellow());
        }
        let count = self.picked_count();
        let size = human(self.picked_size());
        let spans = vec![
            " Selected ".into(),
            if count == 0 { format!("{count}").dim() } else { format!("{count}").green().bold() },
            format!(" of {} items · ", self.gone.iter().filter(|&&gone| !gone).count()).into(),
            if count == 0 { size.dim() } else { size.green().bold() },
            format!(" of {}", human(self.total)).into(),
        ];
        Line::from(spans)
    }

    fn draw_confirm(&self, frame: &mut Frame) {
        let area = frame.area();
        let width = 60.min(area.width.saturating_sub(2));
        let height = 9.min(area.height);
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );

        let button = |label: &'static str, focused: bool, color: Color| {
            if focused {
                Span::styled(
                    format!("  {label}  "),
                    Style::new().fg(color).add_modifier(Modifier::REVERSED | Modifier::BOLD),
                )
            } else {
                Span::styled(format!("  {label}  "), Style::new().dim())
            }
        };
        let size: u64 = self.pending.iter().map(|&i| self.items[i].size).sum();
        let what = match self.pending[..] {
            [only] => fit(&self.items[only].rel.display().to_string(), (width as usize).saturating_sub(14))
                .trim_end()
                .to_string(),
            _ => format!("{} items", self.pending.len()),
        };
        let caveat = if self.dry { "dry run, nothing is deleted" } else { "cannot be undone" };
        let text = vec![
            Line::raw(""),
            Line::from(vec!["Delete ".into(), what.bold(), "?".into()]),
            Line::from(vec!["frees ".dim(), human(size).green().bold(), format!(" · {caveat}").dim()]),
            Line::raw(""),
            Line::from(vec![
                button("Yes", self.confirm_yes, Color::Red),
                "   ".into(),
                button("No", !self.confirm_yes, Color::Reset),
            ]),
            Line::raw(""),
            Line::from("←→ choose · enter confirm".dim()),
        ];
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().red())
            .title(" Confirm ".red().bold());

        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(text).centered().block(block), popup);
    }
}

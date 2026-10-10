use crate::scanner::{Disk, Found, age, human};
use std::time::{Duration, SystemTime};
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
const BAR_WIDTH: usize = 10;

const KEYS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("←→", "fold"),
    ("space/x", "select"),
    ("a", "all"),
    ("e", "expand"),
    ("s", "sort"),
    ("tab", "view"),
    ("enter", "delete"),
    ("q", "quit"),
];

struct App<'a> {
    items: &'a [Found],
    root: &'a str,
    disk: Option<Disk>,
    dry: bool,
    /// Selection per item, shared by both views.
    picked: Vec<bool>,
    trees: [Tree; 2],
    view: usize,
    total: u64,
    /// Rows visible in the list, as of the last draw.
    height: usize,
    confirming: bool,
    /// Which button has focus in the confirmation dialog.
    confirm_delete: bool,
    /// Stalest first rather than biggest first.
    by_age: bool,
    notice: Option<&'static str>,
}

enum Action {
    Quit,
    Delete,
}

/// Runs the interactive picker. Returns the indices of the items the user
/// confirmed for deletion; empty if they quit.
pub fn run(items: &[Found], root: &str, disk: Option<Disk>, dry: bool) -> io::Result<Vec<usize>> {
    let mut app = App {
        items,
        root,
        disk,
        dry,
        picked: vec![false; items.len()],
        trees: [Tree::by_type(items), Tree::by_dir(items)],
        view: BY_TYPE,
        total: items.iter().map(|f| f.size).sum(),
        height: 1,
        confirming: false,
        confirm_delete: false,
        by_age: false,
        notice: None,
    };

    let mut terminal = ratatui::init();
    let action = app.event_loop(&mut terminal);
    ratatui::restore();

    Ok(match action? {
        Action::Quit => Vec::new(),
        Action::Delete => (0..items.len()).filter(|&i| app.picked[i]).collect(),
    })
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
    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> io::Result<Action> {
        loop {
            terminal.draw(|frame| self.draw(frame))?;
            if let Event::Key(key) = event::read()?
                && key.kind == KeyEventKind::Press
                && let Some(action) = self.on_key(key)
            {
                return Ok(action);
            }
        }
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
        if self.confirming {
            match key.code {
                KeyCode::Char('y' | 'Y') => return Some(Action::Delete),
                KeyCode::Enter if self.confirm_delete => return Some(Action::Delete),
                KeyCode::Left | KeyCode::Right | KeyCode::Tab | KeyCode::BackTab | KeyCode::Char('h' | 'l') => {
                    self.confirm_delete = !self.confirm_delete;
                }
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('n' | 'N' | 'q') => self.confirming = false,
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
                let all: Vec<usize> = (0..self.items.len()).collect();
                self.set_picked(&all);
            }
            KeyCode::Tab | KeyCode::BackTab => self.view = 1 - self.view,
            KeyCode::Enter => {
                if self.picked_count() == 0 {
                    self.notice = Some("Nothing selected yet. Press space to select.");
                } else {
                    self.confirming = true;
                    // Start on the harmless choice.
                    self.confirm_delete = false;
                }
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

        if self.confirming {
            self.draw_confirm(frame);
        }
    }

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

        let show_bar = width >= 80;
        let prefix = format!(" {}{arrow} ", "  ".repeat(node.depth));
        let right = META_WIDTH + AGE_WIDTH + SIZE_WIDTH + if show_bar { BAR_WIDTH + 3 } else { 1 };
        let room = width.saturating_sub(prefix.width() + 4 + right);

        let mut spans = vec![
            Span::raw(prefix),
            Span::styled(checkbox, paint(checkbox_style)),
            Span::raw(" "),
            Span::styled(
                fit(&node.label, room),
                paint(if node.leaf.is_none() { Style::new().bold() } else { Style::new() }),
            ),
            Span::styled(format!("{meta:>META_WIDTH$}"), paint(Style::new().dim())),
            Span::styled(format!("{:>AGE_WIDTH$}", age(node.modified)), paint(age_style(node.modified))),
            Span::styled(
                format!("{:>SIZE_WIDTH$}", human(node.size)),
                paint(Style::new().fg(size_color(node.size))),
            ),
        ];
        if show_bar {
            let filled = if self.total == 0 {
                0
            } else {
                ((node.size as f64 / self.total as f64) * BAR_WIDTH as f64).ceil() as usize
            };
            let filled = filled.min(BAR_WIDTH);
            spans.push(Span::raw("  "));
            spans.push(Span::styled("█".repeat(filled), paint(Style::new().blue())));
            spans.push(Span::styled("░".repeat(BAR_WIDTH - filled), paint(Style::new().dim())));
        }
        spans.push(Span::raw(" "));

        let line = Line::from(spans);
        if is_cursor { line.style(Style::new().add_modifier(Modifier::REVERSED)) } else { line }
    }

    fn status_line(&self) -> Line<'static> {
        if let Some(notice) = self.notice {
            return Line::from(format!(" {notice}").yellow());
        }
        let count = self.picked_count();
        let size = human(self.picked_size());
        let spans = vec![
            " Selected ".into(),
            if count == 0 { format!("{count}").dim() } else { format!("{count}").green().bold() },
            format!(" of {} items · ", self.items.len()).into(),
            if count == 0 { size.dim() } else { size.green().bold() },
            format!(" of {}", human(self.total)).into(),
        ];
        Line::from(spans)
    }

    fn draw_confirm(&self, frame: &mut Frame) {
        let area = frame.area();
        let width = 54.min(area.width.saturating_sub(2));
        let height = 9.min(area.height);
        let popup = Rect::new(
            area.x + (area.width - width) / 2,
            area.y + (area.height - height) / 2,
            width,
            height,
        );

        let button = |label: &'static str, focused: bool, color: Color| {
                if focused {
                    Span::styled(format!("  {label}  "), Style::new().fg(color).add_modifier(Modifier::REVERSED | Modifier::BOLD))
                } else {
                    Span::styled(format!("  {label}  "), Style::new().dim())
                }
            };
        let count = self.picked_count();
        let noun = if count == 1 { "item" } else { "items" };
        let text = vec![
            Line::raw(""),
            Line::from(vec![
                "Delete ".into(),
                format!("{count} {noun}").bold(),
                " and free ".into(),
                human(self.picked_size()).green().bold(),
                "?".into(),
            ]),
            Line::from(if self.dry { "dry run, nothing is deleted" } else { "This cannot be undone." }.dim()),
            Line::raw(""),
            Line::from(vec![
                button("Cancel", !self.confirm_delete, Color::Reset),
                "   ".into(),
                button("Delete", self.confirm_delete, Color::Red),
            ]),
            Line::raw(""),
            Line::from("←→ choose · enter confirm · y/n".dim()),
        ];
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().red())
            .title(" Confirm ".red().bold());

        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(text).centered().block(block), popup);
    }
}

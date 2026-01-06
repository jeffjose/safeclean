use crate::projects::ProjectType;
use crate::scanner::FoundDir;
use console::{style, Key, Term};
use std::collections::HashMap;
use std::io;

#[derive(Debug, Clone)]
pub struct GroupedItem {
    pub dir: FoundDir,
    pub selected: bool,
}

#[derive(Debug)]
pub struct Group {
    pub project_type: ProjectType,
    pub items: Vec<GroupedItem>,
    pub collapsed: bool,
}

impl Group {
    pub fn total_size(&self) -> u64 {
        self.items.iter().map(|i| i.dir.size_bytes).sum()
    }

    pub fn all_selected(&self) -> bool {
        self.items.iter().all(|i| i.selected)
    }

    pub fn none_selected(&self) -> bool {
        self.items.iter().all(|i| !i.selected)
    }

    pub fn toggle_all(&mut self) {
        let new_state = !self.all_selected();
        for item in &mut self.items {
            item.selected = new_state;
        }
    }
}

pub struct GroupedSelector {
    groups: Vec<Group>,
    cursor: usize,
    max_path_len: usize,
}

enum CursorPosition {
    GroupHeader(usize),
    Item(usize, usize),
}

impl GroupedSelector {
    pub fn new(found: Vec<FoundDir>) -> Self {
        let mut by_type: HashMap<ProjectType, Vec<FoundDir>> = HashMap::new();

        for dir in found {
            by_type.entry(dir.project_type).or_default().push(dir);
        }

        let max_path_len = by_type
            .values()
            .flat_map(|v| v.iter())
            .map(|d| d.path.display().to_string().len())
            .max()
            .unwrap_or(50);

        let type_order = ProjectType::all();
        let mut groups: Vec<Group> = Vec::new();

        for pt in type_order {
            if let Some(dirs) = by_type.remove(&pt) {
                let items = dirs
                    .into_iter()
                    .map(|dir| GroupedItem { dir, selected: true })
                    .collect();
                groups.push(Group {
                    project_type: pt,
                    items,
                    collapsed: false,
                });
            }
        }

        Self {
            groups,
            cursor: 0,
            max_path_len,
        }
    }

    fn total_lines(&self) -> usize {
        self.groups
            .iter()
            .map(|g| {
                if g.collapsed {
                    1
                } else {
                    1 + g.items.len()
                }
            })
            .sum()
    }

    fn cursor_position(&self) -> CursorPosition {
        let mut line = 0;
        for (gi, group) in self.groups.iter().enumerate() {
            if line == self.cursor {
                return CursorPosition::GroupHeader(gi);
            }
            line += 1;
            if !group.collapsed {
                for ii in 0..group.items.len() {
                    if line == self.cursor {
                        return CursorPosition::Item(gi, ii);
                    }
                    line += 1;
                }
            }
        }
        CursorPosition::GroupHeader(0)
    }

    fn format_size(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if bytes >= GB {
            format!("{:.1} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.1} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.1} KB", bytes as f64 / KB as f64)
        } else {
            format!("{} B", bytes)
        }
    }

    fn render(&self, term: &Term) -> io::Result<()> {
        let mut output = String::new();

        for (gi, group) in self.groups.iter().enumerate() {
            let is_group_cursor = matches!(self.cursor_position(), CursorPosition::GroupHeader(i) if i == gi);

            // Group header
            let checkbox = if group.all_selected() {
                style("[✓]").green()
            } else if group.none_selected() {
                style("[ ]").dim()
            } else {
                style("[~]").yellow()
            };

            let collapse_indicator = if group.collapsed { "▶" } else { "▼" };

            let header = format!(
                "{} {} {} ({} items, {})",
                checkbox,
                collapse_indicator,
                group.project_type.name(),
                group.items.len(),
                Self::format_size(group.total_size())
            );

            if is_group_cursor {
                output.push_str(&format!("{}\n", style(header).reverse()));
            } else {
                output.push_str(&format!("{}\n", style(header).bold()));
            }

            // Items (if not collapsed)
            if !group.collapsed {
                for (ii, item) in group.items.iter().enumerate() {
                    let is_item_cursor =
                        matches!(self.cursor_position(), CursorPosition::Item(g, i) if g == gi && i == ii);

                    let checkbox = if item.selected {
                        style("  [✓]").green()
                    } else {
                        style("  [ ]").dim()
                    };

                    let path_str = item.dir.path.display().to_string();
                    let size_str = item.dir.size_human();

                    let line = format!(
                        "{} {:<width$}  {:>10}",
                        checkbox,
                        path_str,
                        size_str,
                        width = self.max_path_len
                    );

                    if is_item_cursor {
                        output.push_str(&format!("{}\n", style(line).reverse()));
                    } else {
                        output.push_str(&format!("{}\n", line));
                    }
                }
            }
        }

        // Instructions
        output.push_str(&format!(
            "\n{} navigate  {} toggle  {} expand/collapse  {} confirm\n",
            style("↑↓").cyan(),
            style("Space").cyan(),
            style("Tab").cyan(),
            style("Enter").cyan()
        ));

        term.clear_screen()?;
        term.write_str(&output)?;

        Ok(())
    }

    fn move_up(&mut self) {
        if self.cursor > 0 {
            self.cursor -= 1;
        }
    }

    fn move_down(&mut self) {
        let total = self.total_lines();
        if self.cursor + 1 < total {
            self.cursor += 1;
        }
    }

    fn toggle_current(&mut self) {
        match self.cursor_position() {
            CursorPosition::GroupHeader(gi) => {
                self.groups[gi].toggle_all();
            }
            CursorPosition::Item(gi, ii) => {
                self.groups[gi].items[ii].selected = !self.groups[gi].items[ii].selected;
            }
        }
    }

    fn toggle_collapse(&mut self) {
        if let CursorPosition::GroupHeader(gi) = self.cursor_position() {
            self.groups[gi].collapsed = !self.groups[gi].collapsed;
        }
    }

    pub fn run(mut self) -> io::Result<Vec<FoundDir>> {
        let term = Term::stderr();
        term.hide_cursor()?;

        loop {
            self.render(&term)?;

            match term.read_key()? {
                Key::ArrowUp | Key::Char('k') => self.move_up(),
                Key::ArrowDown | Key::Char('j') => self.move_down(),
                Key::Char(' ') => self.toggle_current(),
                Key::Tab => self.toggle_collapse(),
                Key::Enter => break,
                Key::Escape | Key::Char('q') => {
                    term.show_cursor()?;
                    term.clear_screen()?;
                    return Ok(Vec::new());
                }
                _ => {}
            }
        }

        term.show_cursor()?;
        term.clear_screen()?;

        let selected: Vec<FoundDir> = self
            .groups
            .into_iter()
            .flat_map(|g| g.items)
            .filter(|i| i.selected)
            .map(|i| i.dir)
            .collect();

        Ok(selected)
    }
}

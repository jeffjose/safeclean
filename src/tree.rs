use crate::projects::ProjectType;
use crate::scanner::Found;
use std::cmp::Reverse;
use std::collections::BTreeMap;
use std::path::Component;
use std::time::SystemTime;

pub struct Node {
    pub label: String,
    /// The project type for a type header or a leaf.
    pub tag: Option<ProjectType>,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Index into the scanned items; set only on leaves.
    pub leaf: Option<usize>,
    pub depth: usize,
    pub expanded: bool,
    pub size: u64,
    /// Most recent activity of anything beneath.
    pub modified: Option<SystemTime>,
    /// Number of leaves beneath (1 for a leaf).
    pub count: usize,
    /// Number of those leaves currently selected.
    pub picked: usize,
}

/// A collapsible view over the scanned items. Nodes live in an arena where a
/// parent always has a lower index than its children.
pub struct Tree {
    pub nodes: Vec<Node>,
    roots: Vec<usize>,
    /// Node ids currently visible, top to bottom.
    pub rows: Vec<usize>,
    pub cursor: usize,
    pub offset: usize,
}

struct Draft {
    label: String,
    tag: Option<ProjectType>,
    leaf: Option<usize>,
    children: Vec<Draft>,
}

type Entry<'a> = (usize, &'a [String]);

/// Builds a path trie, folding chains of single-child directories into one
/// node so `a/b/c/target` is one row rather than four.
fn trie(entries: Vec<Entry>, items: &[Found]) -> Vec<Draft> {
    let mut groups: BTreeMap<&str, Vec<Entry>> = BTreeMap::new();
    for (idx, components) in entries {
        if let Some((first, rest)) = components.split_first() {
            groups.entry(first.as_str()).or_default().push((idx, rest));
        }
    }

    groups
        .into_iter()
        .map(|(name, group)| {
            if let Some(&(idx, _)) = group.iter().find(|(_, rest)| rest.is_empty()) {
                return Draft {
                    label: name.to_string(),
                    tag: Some(items[idx].kind),
                    leaf: Some(idx),
                    children: Vec::new(),
                };
            }
            let mut children = trie(group, items);
            if children.len() == 1 {
                let only = children.pop().unwrap();
                Draft { label: join(name, &only.label), ..only }
            } else {
                Draft { label: join(name, ""), tag: None, leaf: None, children }
            }
        })
        .collect()
}

/// The filesystem root is carried as an empty component so that joining it
/// to what follows reads `/tmp`, not `//tmp`.
fn component(part: Component) -> String {
    match part {
        Component::RootDir => String::new(),
        other => other.as_os_str().to_string_lossy().into_owned(),
    }
}

fn join(parent: &str, child: &str) -> String {
    match (parent, child) {
        ("", "") => "/".to_string(),
        (_, "") => parent.to_string(),
        _ => format!("{parent}/{child}"),
    }
}

fn components(items: &[Found]) -> Vec<Vec<String>> {
    items
        .iter()
        .map(|f| f.rel.components().map(component).collect())
        .collect()
}

impl Tree {
    /// Project type at the top level, directories beneath.
    pub fn by_type(items: &[Found]) -> Self {
        let paths = components(items);
        let drafts = ProjectType::ALL
            .iter()
            .filter_map(|&kind| {
                let entries: Vec<Entry> = items
                    .iter()
                    .enumerate()
                    .filter(|(_, f)| f.kind == kind)
                    .map(|(i, _)| (i, paths[i].as_slice()))
                    .collect();
                if entries.is_empty() {
                    return None;
                }
                let mut label = kind.name().to_string();
                let mut children = trie(entries, items);
                // When everything of a kind sits under one directory, name it
                // in the header instead of spending a level on it.
                if children.len() == 1 && children[0].leaf.is_none() {
                    let only = children.pop().unwrap();
                    label = format!("{label} · {}", only.label);
                    children = only.children;
                }
                Some(Draft { label, tag: Some(kind), leaf: None, children })
            })
            .collect();
        Self::build(drafts, items)
    }

    /// Plain directory hierarchy, regardless of project type.
    pub fn by_dir(items: &[Found]) -> Self {
        let paths = components(items);
        let entries = paths.iter().enumerate().map(|(i, p)| (i, p.as_slice())).collect();
        Self::build(trie(entries, items), items)
    }

    fn build(drafts: Vec<Draft>, items: &[Found]) -> Self {
        let mut tree = Tree { nodes: Vec::new(), roots: Vec::new(), rows: Vec::new(), cursor: 0, offset: 0 };
        let mut roots: Vec<usize> = drafts.into_iter().map(|d| tree.add(d, None, 0, items)).collect();
        roots.sort_by_key(|&id| Reverse(tree.nodes[id].size));
        tree.roots = roots;
        tree.refresh();
        tree
    }

    fn add(&mut self, draft: Draft, parent: Option<usize>, depth: usize, items: &[Found]) -> usize {
        let id = self.nodes.len();
        self.nodes.push(Node {
            label: draft.label,
            tag: draft.tag,
            parent,
            children: Vec::new(),
            leaf: draft.leaf,
            depth,
            expanded: false,
            size: draft.leaf.map_or(0, |i| items[i].size),
            modified: draft.leaf.and_then(|i| items[i].modified),
            count: draft.leaf.is_some() as usize,
            picked: 0,
        });
        let mut children: Vec<usize> =
            draft.children.into_iter().map(|c| self.add(c, Some(id), depth + 1, items)).collect();
        // Biggest first; the trie's alphabetical order breaks ties.
        children.sort_by_key(|&c| Reverse(self.nodes[c].size));
        self.nodes[id].size += children.iter().map(|&c| self.nodes[c].size).sum::<u64>();
        self.nodes[id].count += children.iter().map(|&c| self.nodes[c].count).sum::<usize>();
        self.nodes[id].modified = children.iter().map(|&c| self.nodes[c].modified).max().unwrap_or(self.nodes[id].modified);
        self.nodes[id].children = children;
        id
    }

    fn refresh(&mut self) {
        fn walk(tree: &Tree, id: usize, rows: &mut Vec<usize>) {
            rows.push(id);
            if tree.nodes[id].expanded {
                for &child in &tree.nodes[id].children {
                    walk(tree, child, rows);
                }
            }
        }
        let mut rows = Vec::new();
        for &root in &self.roots {
            walk(self, root, &mut rows);
        }
        self.rows = rows;
        self.cursor = self.cursor.min(self.rows.len().saturating_sub(1));
    }

    /// Reorders every level: stalest first if `by_age`, else biggest first.
    pub fn sort(&mut self, by_age: bool) {
        let current = self.current();
        let nodes = &self.nodes;
        let order = |ids: &mut Vec<usize>| {
            if by_age {
                ids.sort_by_key(|&id| (nodes[id].modified, Reverse(nodes[id].size)));
            } else {
                ids.sort_by_key(|&id| Reverse(nodes[id].size));
            }
        };
        let mut roots = std::mem::take(&mut self.roots);
        order(&mut roots);
        let mut sorted: Vec<Vec<usize>> = nodes.iter().map(|node| node.children.clone()).collect();
        sorted.iter_mut().for_each(order);
        self.roots = roots;
        for (node, children) in self.nodes.iter_mut().zip(sorted) {
            node.children = children;
        }
        self.refresh();
        self.focus(current);
    }

    pub fn current(&self) -> usize {
        self.rows[self.cursor]
    }

    /// Puts the cursor on `id`, or on its nearest visible ancestor.
    fn focus(&mut self, mut id: usize) {
        loop {
            if let Some(row) = self.rows.iter().position(|&r| r == id) {
                self.cursor = row;
                return;
            }
            match self.nodes[id].parent {
                Some(parent) => id = parent,
                None => return,
            }
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.cursor = self.cursor.saturating_add_signed(delta).min(last);
    }

    pub fn move_to_end(&mut self) {
        self.cursor = self.rows.len().saturating_sub(1);
    }

    /// Flips the current node between expanded and collapsed.
    pub fn toggle(&mut self) {
        let id = self.current();
        if !self.nodes[id].children.is_empty() {
            self.nodes[id].expanded = !self.nodes[id].expanded;
            self.refresh();
        }
    }

    /// Expands the current node, or steps into it if already expanded.
    pub fn expand(&mut self) {
        let id = self.current();
        if self.nodes[id].children.is_empty() {
            return;
        }
        if self.nodes[id].expanded {
            self.cursor += 1;
        } else {
            self.nodes[id].expanded = true;
            self.refresh();
        }
    }

    /// Collapses the current node, or jumps to its parent if there is
    /// nothing to collapse.
    pub fn collapse(&mut self) {
        let id = self.current();
        if self.nodes[id].expanded {
            self.nodes[id].expanded = false;
            self.refresh();
        } else if let Some(parent) = self.nodes[id].parent {
            self.focus(parent);
        }
    }

    pub fn all_expanded(&self) -> bool {
        self.nodes.iter().all(|n| n.expanded || n.children.is_empty())
    }

    pub fn set_all_expanded(&mut self, expanded: bool) {
        let id = self.current();
        for node in &mut self.nodes {
            node.expanded = expanded && !node.children.is_empty();
        }
        self.refresh();
        self.focus(id);
    }

    /// Item indices of every leaf at or beneath `id`.
    pub fn leaves(&self, id: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![id];
        while let Some(id) = stack.pop() {
            let node = &self.nodes[id];
            out.extend(node.leaf);
            stack.extend(&node.children);
        }
        out
    }

    /// Recomputes how many leaves are selected under every node.
    pub fn recount(&mut self, picked: &[bool]) {
        for id in (0..self.nodes.len()).rev() {
            let own = self.nodes[id].leaf.map_or(0, |i| picked[i] as usize);
            let below: usize = self.nodes[id].children.iter().map(|&c| self.nodes[c].picked).sum();
            self.nodes[id].picked = own + below;
        }
    }
}

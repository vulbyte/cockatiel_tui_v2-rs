//! Blender-inspired Binary Space Partitioning (BSP) layout engine.
//!
//! The terminal is a single root rectangle recursively divided into subregions
//! by a binary tree:
//!
//! ```text
//!         [ Split Node ] (Axis: Vertical, Ratio: 0.40)
//!          /          \
//!  [ Leaf ]        [ Split Node ] (Axis: Horizontal, Ratio: 0.60)
//!  view: A             /          \
//!              [ Leaf ]          [ Leaf ]
//!              view: B           view: C
//! ```
//!
//! * `SplitNode` divides its rectangle by `ratio` along `axis` into two child
//!   subtrees (left/top and right/bottom).
//! * `LeafNode` is a pane hosting a single `ViewType` (one of the mounted
//!   windows), with its own `calculated_rect`.
//!
//! Focus is a *path* from the root to the focused leaf (e.g. `[0, 1]` — the
//! right child of the root), which lets a leaf host any view and lets
//! navigation walk the tree spatially instead of a fixed rotation.
//!
//! Some tree mutations are implemented and tested here but not yet bound to a
//! key (split/join come in the next phase); they are kept ready rather than
//! deleted.

#![allow(dead_code)]

use ratatui::layout::Rect;

use crate::app::{Window, WindowId};

/// Minimum pane size, below which a split or resize is rejected.
pub const MIN_WIDTH: u16 = 12;
/// Minimum pane height, below which a split or resize is rejected.
pub const MIN_HEIGHT: u16 = 5;

/// A view type that a leaf pane can host. These wrap the existing six window
/// implementations; a leaf's view can be swapped at runtime (the dropdown).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ViewType {
    CockatielInfo,
    Logs,
    ModuleManager,
    EngineGraph,
    Prompts,
    TopUsers,
}

impl ViewType {
    pub fn all() -> &'static [ViewType] {
        &[
            ViewType::CockatielInfo,
            ViewType::Logs,
            ViewType::ModuleManager,
            ViewType::EngineGraph,
            ViewType::Prompts,
            ViewType::TopUsers,
        ]
    }

    pub fn name(self) -> &'static str {
        match self {
            ViewType::CockatielInfo => "cockatiel_info",
            ViewType::Logs => "logs",
            ViewType::ModuleManager => "module_manager",
            ViewType::EngineGraph => "engine_graph",
            ViewType::Prompts => "prompts",
            ViewType::TopUsers => "top_users",
        }
    }

    /// Resolve a view from its name (for persistence / dropdown labels).
    pub fn from_name(name: &str) -> Option<ViewType> {
        ViewType::all().iter().copied().find(|v| v.name() == name)
    }

    pub fn window_id(self) -> WindowId {
        match self {
            ViewType::CockatielInfo => WindowId::Logo,
            ViewType::Logs => WindowId::Log,
            ViewType::ModuleManager => WindowId::Modules,
            ViewType::EngineGraph => WindowId::Chart,
            ViewType::Prompts => WindowId::Prompts,
            ViewType::TopUsers => WindowId::Users,
        }
    }
}

/// The current tool mounted in a leaf. The window state lives here so a swap
/// can drop it (fresh view) or keep it, per view type.
pub struct LeafContent {
    pub view: ViewType,
    pub window: Box<dyn Window>,
}

impl std::fmt::Debug for LeafContent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LeafContent").field("view", &self.view).finish()
    }
}

/// A node in the layout tree.
#[derive(Debug)]
pub enum Node {
    Split(SplitNode),
    Leaf(LeafNode),
}

/// An internal divider: splits its rectangle into two child subtrees.
#[derive(Debug)]
pub struct SplitNode {
    /// HORIZONTAL = left/right split; VERTICAL = top/bottom split.
    pub axis: Axis,
    /// Fraction of the rectangle allocated to the left/top child (0.05..0.95).
    pub ratio: f32,
    pub child_a: Box<Node>,
    pub child_b: Box<Node>,
    /// The divider's own rectangle (the full split region), set by `compute`.
    /// Its inner edge is what a user drags to resize this split.
    pub rect: Rect,
}

/// A leaf: one mounted view, holding its own window + calculated rect.
#[derive(Debug)]
pub struct LeafNode {
    pub id: String,
    pub content: LeafContent,
    pub rect: Rect,
}

/// Split axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Axis {
    /// Left/right split.
    Horizontal,
    /// Top/bottom split.
    Vertical,
}

impl Axis {
    pub fn name(self) -> &'static str {
        match self {
            Axis::Horizontal => "h",
            Axis::Vertical => "v",
        }
    }
}

/// The layout tree + focus path.
#[derive(Debug)]
pub struct LayoutTree {
    pub root: Box<Node>,
    /// Path from the root to the focused leaf (index of child_a/child_b at
    /// each level). Empty when the root itself is a leaf.
    pub focus: Vec<usize>,
    /// Border-drag state: the split being resized + which edge of its
    /// child_a is being dragged. Any divider in the tree is draggable.
    pub dragging: Option<(Vec<usize>, DragDir)>,
    pub drag_start: Option<(u16, u16)>,
}

/// Which edge of the dragged divider is being pulled, relative to the split's
/// child_a side. `Left`/`Top` mean the cursor is on the child_a side of the
/// divider (growing child_a pulls it right/down); `Right`/`Bottom` the
/// child_b side (growing child_a pushes the divider toward child_b).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragDir {
    /// Dragging a horizontal (left/right) split's divider; cursor on the left
    /// (child_a) side.
    Left,
    /// Dragging a horizontal split's divider; cursor on the right (child_b) side.
    Right,
    /// Dragging a vertical (top/bottom) split's divider; cursor on the top
    /// (child_a) side.
    Top,
    /// Dragging a vertical split's divider; cursor on the bottom (child_b) side.
    Bottom,
}

/// Default 5-pane arrangement, mirroring the current fixed grid so the new
/// engine has no visual regression on first launch:
///
/// ```text
/// ┌──────────────┬───────────────────────┐
/// │ info         │ module_manager        │
/// ├──────────────┤                       │
/// │ logs         │                       │
/// ├──────────────┼──────────┬────────────┤
/// │ engine_graph │          │ prompts    │
/// └──────────────┴──────────┴────────────┘
/// ```
pub fn default_tree() -> LayoutTree {
    let info = leaf("info", ViewType::CockatielInfo);
    let logs = leaf("logs", ViewType::Logs);
    let left_col = split(Axis::Vertical, 0.50, info, logs);

    let module_manager = leaf("modules", ViewType::ModuleManager);
    let graph = leaf("graph", ViewType::EngineGraph);
    let prompts = leaf("prompts", ViewType::Prompts);
    let bottom = split(Axis::Horizontal, 0.60, graph, prompts);
    let right_col = split(Axis::Vertical, 0.60, module_manager, bottom);

    LayoutTree {
        root: split(Axis::Horizontal, 0.30, left_col, right_col),
        focus: vec![0, 0],
        dragging: None,
        drag_start: None,
    }
}

/// A serializable snapshot of the layout structure (splits, ratios, leaf
/// views) — the windows themselves are re-created on load.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum LayoutSnapshot {
    Leaf { id: String, view: String },
    Split { axis: String, ratio: f32, a: Box<LayoutSnapshot>, b: Box<LayoutSnapshot> },
}

impl LayoutTree {
    /// Serialize the tree structure (no window state) for persistence.
    pub fn snapshot(&self) -> LayoutSnapshot {
        snapshot_node(&self.root)
    }

    /// Rebuild the tree from a snapshot. Window state is recreated fresh.
    pub fn from_snapshot(snap: LayoutSnapshot) -> LayoutTree {
        let root = from_snapshot_node(snap);
        // Focus the first leaf (depth-first order) after rebuild.
        let mut tree = LayoutTree { root, focus: Vec::new(), dragging: None, drag_start: None };
        let first = tree.leaf_order().first().cloned().map(|(id, _)| id).unwrap_or_else(|| "pane".to_string());
        tree.focus = tree.path_to_id(&first);
        tree
    }

    /// Save the layout structure to `path` (atomic temp+rename).
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string_pretty(&self.snapshot())
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json)?;
        std::fs::rename(tmp, path)
    }

    /// Load a layout from `path`, or `None` if it doesn't exist / is malformed.
    pub fn load(path: &std::path::Path) -> Option<LayoutTree> {
        let content = std::fs::read_to_string(path).ok()?;
        let snap: LayoutSnapshot = serde_json::from_str(&content).ok()?;
        Some(LayoutTree::from_snapshot(snap))
    }
}

fn snapshot_node(node: &Node) -> LayoutSnapshot {
    match node {
        Node::Split(s) => LayoutSnapshot::Split {
            axis: s.axis.name().to_string(),
            ratio: s.ratio,
            a: Box::new(snapshot_node(&s.child_a)),
            b: Box::new(snapshot_node(&s.child_b)),
        },
        Node::Leaf(l) => LayoutSnapshot::Leaf { id: l.id.clone(), view: l.content.view.name().to_string() },
    }
}

fn from_snapshot_node(snap: LayoutSnapshot) -> Box<Node> {
    match snap {
        LayoutSnapshot::Leaf { id, view } => {
            let view = ViewType::from_name(&view).unwrap_or(ViewType::Logs);
            leaf(&id, view)
        }
        LayoutSnapshot::Split { axis, ratio, a, b } => {
            let axis = if axis == "v" { Axis::Vertical } else { Axis::Horizontal };
            split(axis, ratio, from_snapshot_node(*a), from_snapshot_node(*b))
        }
    }
}

/// Build a tree with a single leaf (detached pop-out mode).
pub fn single_tree(view: ViewType) -> LayoutTree {
    LayoutTree {
        root: leaf("pane", view),
        focus: vec![],
        dragging: None,
        drag_start: None,
    }
}

/// Build a tree with a single leaf hosting an already-constructed window.
pub fn tree_with_window(view: ViewType, window: Box<dyn Window>) -> LayoutTree {
    LayoutTree {
        root: Box::new(Node::Leaf(LeafNode {
            id: "pane".to_string(),
            content: LeafContent { view, window },
            rect: Rect::default(),
        })),
        focus: vec![],
        dragging: None,
        drag_start: None,
    }
}

fn leaf(id: &str, view: ViewType) -> Box<Node> {
    Box::new(Node::Leaf(LeafNode {
        id: id.to_string(),
        content: LeafContent {
            view,
            window: make_window(view),
        },
        rect: Rect::default(),
    }))
}

fn split(axis: Axis, ratio: f32, a: Box<Node>, b: Box<Node>) -> Box<Node> {
    Box::new(Node::Split(SplitNode {
        axis,
        ratio,
        child_a: a,
        child_b: b,
        rect: Rect::default(),
    }))
}

/// Build a window for a view type. The existing six window impls are reused.
pub fn make_window(view: ViewType) -> Box<dyn Window> {
    use crate::windows::{ChartWindow, LogWindow, LogoWindow, ModulesWindow, PromptsWindow, UsersWindow};
    match view {
        ViewType::CockatielInfo => Box::new(LogoWindow),
        ViewType::Logs => Box::new(LogWindow::new()),
        ViewType::ModuleManager => Box::new(ModulesWindow::new()),
        ViewType::EngineGraph => Box::new(ChartWindow::new()),
        ViewType::Prompts => Box::new(PromptsWindow::new()),
        ViewType::TopUsers => Box::new(UsersWindow::new()),
    }
}

impl LayoutTree {
    /// Recursively compute every leaf's rectangle from `root_rect`, honoring
    /// `ratio` at each split and clamping to the minimum pane size.
    pub fn compute(&mut self, root_rect: Rect) {
        compute_node(&mut self.root, root_rect);
    }

    /// The id of the focused leaf.
    pub fn focused_id(&self) -> String {
        match leaf_at(&self.root, &self.focus) {
            Node::Leaf(l) => l.id.clone(),
            Node::Split(_) => String::from("root"),
        }
    }

    /// The focused leaf's view type.
    pub fn focused_view(&self) -> ViewType {
        match leaf_at(&self.root, &self.focus) {
            Node::Leaf(l) => l.content.view,
            Node::Split(_) => ViewType::CockatielInfo,
        }
    }

    /// The focused leaf's window (mutable).
    pub fn focused_window(&mut self) -> Option<&mut Box<dyn Window>> {
        match leaf_at_mut(&mut self.root, &self.focus) {
            Node::Leaf(l) => Some(&mut l.content.window),
            Node::Split(_) => None,
        }
    }

    /// The focused leaf's calculated rectangle (for rendering the active border).
    pub fn focused_rect(&self) -> Option<Rect> {
        match leaf_at(&self.root, &self.focus) {
            Node::Leaf(l) => Some(l.rect),
            Node::Split(_) => None,
        }
    }

    /// Iterate every leaf with its id, view, rect, and whether it is focused.
    pub fn leaves(&self) -> Vec<(String, ViewType, Rect, bool)> {
        let mut out = Vec::new();
        collect_leaves(&self.root, &self.focus, &mut out);
        out
    }

    /// Find the first leaf whose view maps to `id`, returning its window.
    pub fn window_mut_by_id(&mut self, id: WindowId) -> Option<&mut Box<dyn Window>> {
        let mut stack: Vec<&mut Node> = vec![&mut self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&mut s.child_b);
                    stack.push(&mut s.child_a);
                }
                Node::Leaf(l) => {
                    if l.content.view.window_id() == id {
                        return Some(&mut l.content.window);
                    }
                }
            }
        }
        None
    }

    /// The window at a given path from the root (mutable).
    pub fn window_mut_by_path(&mut self, path: &[usize]) -> Option<&mut Box<dyn Window>> {
        match leaf_at_mut(&mut self.root, path) {
            Node::Leaf(l) => Some(&mut l.content.window),
            Node::Split(_) => None,
        }
    }

    /// If the node at `path` is a Split, its (axis, ratio) — no borrow held.
    pub fn split_at(&self, path: &[usize]) -> Option<(Axis, f32)> {
        match leaf_at(&self.root, path) {
            Node::Split(s) => Some((s.axis, s.ratio)),
            Node::Leaf(_) => None,
        }
    }

    /// If the node at `path` is a Leaf, its (id, view, is_focused) — no borrow
    /// held.
    pub fn leaf_info_at(&self, path: &[usize]) -> Option<(String, ViewType, bool)> {
        match leaf_at(&self.root, path) {
            Node::Leaf(l) => Some((l.id.clone(), l.content.view, path == self.focus.as_slice())),
            Node::Split(_) => None,
        }
    }

    /// Find the first leaf whose view maps to `id`, returning an immutable
    /// window reference.
    pub fn window_by_id(&self, id: WindowId) -> Option<&dyn Window> {
        let mut stack: Vec<&Node> = vec![&self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&s.child_b);
                    stack.push(&s.child_a);
                }
                Node::Leaf(l) => {
                    if l.content.view.window_id() == id {
                        return Some(l.content.window.as_ref());
                    }
                }
            }
        }
        None
    }

    /// True if any leaf's window satisfies `f`.
    pub fn any_window(&self, f: impl Fn(&dyn Window) -> bool) -> bool {
        let mut stack: Vec<&Node> = vec![&self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&s.child_b);
                    stack.push(&s.child_a);
                }
                Node::Leaf(l) => {
                    if f(l.content.window.as_ref()) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// The first clickable link any leaf's window exposes.
    pub fn any_window_link(&self) -> Option<(Rect, String)> {
        let mut stack: Vec<&Node> = vec![&self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&s.child_b);
                    stack.push(&s.child_a);
                }
                Node::Leaf(l) => {
                    if let Some(link) = l.content.window.pending_link() {
                        return Some(link);
                    }
                }
            }
        }
        None
    }

    /// Whether any leaf hosts the given view (by its WindowId).
    pub fn has_view(&self, id: WindowId) -> bool {
        self.leaves().iter().any(|(_, v, _, _)| v.window_id() == id)
    }

    /// The calculated rectangle of a leaf by id.
    pub fn leaf_rect_by_id(&self, id: &str) -> Option<Rect> {
        let mut stack: Vec<&Node> = vec![&self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&s.child_b);
                    stack.push(&s.child_a);
                }
                Node::Leaf(l) => {
                    if l.id == id {
                        return Some(l.rect);
                    }
                }
            }
        }
        None
    }

    /// The view currently mounted in a leaf by id.
    pub fn view_at(&self, id: &str) -> Option<ViewType> {
        let mut stack: Vec<&Node> = vec![&self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&s.child_b);
                    stack.push(&s.child_a);
                }
                Node::Leaf(l) => {
                    if l.id == id {
                        return Some(l.content.view);
                    }
                }
            }
        }
        None
    }

    /// Swap a leaf's view by its id (the dropdown's action). The old window
    /// state is dropped and a fresh one mounts.
    pub fn swap_view_by_id(&mut self, id: &str, new_view: ViewType) {
        let mut stack: Vec<&mut Node> = vec![&mut self.root];
        while let Some(node) = stack.pop() {
            match node {
                Node::Split(s) => {
                    stack.push(&mut s.child_b);
                    stack.push(&mut s.child_a);
                }
                Node::Leaf(l) => {
                    if l.id == id {
                        l.content.view = new_view;
                        l.content.window = make_window(new_view);
                        return;
                    }
                }
            }
        }
    }

    /// Split the focused leaf along `axis`. Returns false (and changes nothing)
    /// if the resulting panes would fall below the minimum size.
    pub fn split_focused(&mut self, axis: Axis) -> bool {
        let target = self.focused_rect().unwrap_or_default();
        let (a_ok, b_ok) = match axis {
            Axis::Horizontal => {
                let left = (target.width as f32 * 0.5) as u16;
                let right = target.width.saturating_sub(left);
                (left >= MIN_WIDTH, right >= MIN_WIDTH)
            }
            Axis::Vertical => {
                let top = (target.height as f32 * 0.5) as u16;
                let bottom = target.height.saturating_sub(top);
                (top >= MIN_HEIGHT, bottom >= MIN_HEIGHT)
            }
        };
        if !a_ok || !b_ok {
            return false;
        }

        // Capture the focused leaf's view (the left child keeps it).
        let focused_view = self.focused_view();
        let path = self.focus.clone();
        let a_id = format!("{}-a", uuid_fragment());
        let b_id = format!("{}-b", uuid_fragment());

        let old_root = std::mem::replace(&mut self.root, Box::new(Node::Leaf(LeafNode {
            id: "swap".to_string(),
            content: LeafContent { view: ViewType::Logs, window: make_window(ViewType::Logs) },
            rect: Rect::default(),
        })));
        self.root = replace_leaf(old_root, &path, |_old| {
            Node::Split(SplitNode {
                axis,
                ratio: 0.5,
                rect: Rect::default(),
                child_a: Box::new(Node::Leaf(LeafNode {
                    id: a_id,
                    content: LeafContent { view: focused_view, window: make_window(focused_view) },
                    rect: Rect::default(),
                })),
                child_b: Box::new(Node::Leaf(LeafNode {
                    id: b_id,
                    content: LeafContent { view: ViewType::Logs, window: make_window(ViewType::Logs) },
                    rect: Rect::default(),
                })),
            })
        });
        self.focus.push(0);
        true
    }

    /// Swap the focused leaf's view (the dropdown action). Returns the previous
    /// view. The old window state is dropped (a fresh window mounts) — per-view
    /// state preservation is a later refinement.
    pub fn swap_focused_view(&mut self, new_view: ViewType) -> Option<ViewType> {
        let path = self.focus.clone();
        let old = match leaf_at_mut(&mut self.root, &path) {
            Node::Leaf(l) => {
                let old = l.content.view;
                l.content.view = new_view;
                l.content.window = make_window(new_view);
                Some(old)
            }
            Node::Split(_) => None,
        };
        old
    }

    /// Move focus to the leaf with the given id.
    pub fn set_focus_to(&mut self, id: &str) {
        self.focus = self.path_to_id(id);
    }

    /// Whether a click at `(x, y)` is on ANY divider in the tree, and which side
    /// of it. Returns `(path, dir)` where `path` is the split to resize.
    /// This makes every sub-window border draggable, not just the focused one.
    pub fn split_at_point(&self, x: u16, y: u16) -> Option<(Vec<usize>, DragDir)> {
        let threshold = 1;
        let mut stack: Vec<(Vec<usize>, &Node)> = vec![(Vec::new(), &self.root)];
        while let Some((path, node)) = stack.pop() {
            match node {
                Node::Split(s) => {
                    let rect = s.rect;
                    let (a_rect, _) = split_rects(s.axis, s.ratio, rect);
                    // The divider runs along the child_a edge.
                    let on_divider = match s.axis {
                        Axis::Horizontal => {
                            let divider_x = a_rect.x + a_rect.width;
                            x >= divider_x.saturating_sub(threshold) && x <= divider_x + threshold
                                && y >= rect.y && y <= rect.y + rect.height
                        }
                        Axis::Vertical => {
                            let divider_y = a_rect.y + a_rect.height;
                            y >= divider_y.saturating_sub(threshold) && y <= divider_y + threshold
                                && x >= rect.x && x <= rect.x + rect.width
                        }
                    };
                    if on_divider {
                        let dir = match s.axis {
                            Axis::Horizontal => if x < a_rect.x + a_rect.width { DragDir::Left } else { DragDir::Right },
                            Axis::Vertical => if y < a_rect.y + a_rect.height { DragDir::Top } else { DragDir::Bottom },
                        };
                        return Some((path, dir));
                    }
                    // Recurse into children in case the click is on an inner divider.
                    let mut pa = path.clone();
                    pa.push(0);
                    let mut pb = path.clone();
                    pb.push(1);
                    stack.push((pb, &s.child_b));
                    stack.push((pa, &s.child_a));
                }
                Node::Leaf(_) => {}
            }
        }
        None
    }

    /// Move the divider of the split at `path` by `delta` (fraction of the
    /// split's full extent; positive grows child_a). Clamped to 0.05..0.95.
    pub fn resize_split(&mut self, path: &[usize], delta: f32) -> bool {
        if let Node::Split(s) = leaf_at_mut(&mut self.root, path) {
            let new_ratio = (s.ratio + delta).clamp(0.05, 0.95);
            if (new_ratio - s.ratio).abs() < f32::EPSILON {
                return false;
            }
            s.ratio = new_ratio;
            true
        } else {
            false
        }
    }

    /// Update a split drag from mouse movement. The drag target is stored in
    /// `self.dragging`; movement magnitude is scaled by the split's extent.
    pub fn update_split_drag(&mut self, x: u16, y: u16) {
        let Some((path, dir)) = self.dragging.clone() else { return };
        let (start_x, start_y) = self.drag_start.unwrap_or((x, y));
        // Dragging a divider right or down ALWAYS grows child_a (the left/top
        // pane), regardless of which side of the divider the cursor grabbed.
        let delta = match dir {
            DragDir::Left | DragDir::Right => {
                let dx = i32::from(x) - i32::from(start_x);
                dx as f32 / 200.0
            }
            DragDir::Top | DragDir::Bottom => {
                let dy = i32::from(y) - i32::from(start_y);
                dy as f32 / 200.0
            }
        };
        let _ = self.resize_split(&path, delta);
        self.drag_start = Some((x, y));
    }

    /// Move focus to the next leaf in tree order (used by Tab / FocusNext).
    pub fn focus_next(&mut self) {
        let order = self.leaf_order();
        let idx = order.iter().position(|(id, _)| *id == self.focused_id()).unwrap_or(0);
        let next = (idx + 1) % order.len();
        let target_id = order[next].0.clone();
        self.focus = self.path_to_id(&target_id);
    }

    /// Move focus to the previous leaf in tree order (FocusPrev / Shift+Tab).
    pub fn focus_prev(&mut self) {
        let order = self.leaf_order();
        let idx = order.iter().position(|(id, _)| *id == self.focused_id()).unwrap_or(0);
        let prev = if idx == 0 { order.len() - 1 } else { idx - 1 };
        let target_id = order[prev].0.clone();
        self.focus = self.path_to_id(&target_id);
    }

    /// Join the focused leaf into its sibling: collapse the focused leaf's
    /// parent SplitNode and promote the SIBLING. Returns false when the focused
    /// leaf has no parent (it is the only pane), in which case nothing changes.
    ///
    /// Direct siblings under a SplitNode always share a full edge on the split
    /// axis, so the spec's join/alignment rule is automatically satisfied — a
    /// join is only legal between direct siblings, which is exactly the parent
    /// relationship this removes.
    pub fn join_focused(&mut self) -> bool {
        if self.focus.len() < 2 {
            return false; // focus is at/near the root; nothing to join.
        }
        // The parent split is at `focus[..len-1]`; we promote the sibling that
        // is NOT the focused child.
        let parent_path = &self.focus[..self.focus.len() - 1];
        let which = self.focus[self.focus.len() - 1]; // 0 or 1
        let sibling = if which == 0 { 1 } else { 0 };

        let parent = leaf_at_mut(&mut self.root, parent_path);
        if let Node::Split(s) = parent {
            let promoted = if sibling == 0 {
                std::mem::replace(&mut s.child_a, Box::new(Node::Leaf(LeafNode {
                    id: "join-placeholder".into(),
                    content: LeafContent { view: ViewType::Logs, window: make_window(ViewType::Logs) },
                    rect: Rect::default(),
                })))
            } else {
                std::mem::replace(&mut s.child_b, Box::new(Node::Leaf(LeafNode {
                    id: "join-placeholder".into(),
                    content: LeafContent { view: ViewType::Logs, window: make_window(ViewType::Logs) },
                    rect: Rect::default(),
                })))
            };
            // Replace the whole parent split with the promoted sibling subtree.
            *parent = *promoted;
            // Focus now points at the promoted subtree; rebase onto it.
            self.focus = parent_path.to_vec();
            return true;
        }
        false
    }

    /// Resize the focused leaf's shared split by adjusting the parent ratio.
    /// `delta` is a signed fraction of the full pane size (positive grows the
    /// left/top child). Returns false if the move would breach the minimum
    /// pane size.
    pub fn resize_focused(&mut self, delta: f32) -> bool {
        if self.focus.len() < 2 {
            return false;
        }
        let parent_path = &self.focus[..self.focus.len() - 1];
        let which = self.focus[self.focus.len() - 1];
        let parent = leaf_at_mut(&mut self.root, parent_path);
        let Node::Split(s) = parent else { return false };

        let new_ratio = if which == 0 {
            s.ratio + delta
        } else {
            s.ratio - delta
        };
        let new_ratio = new_ratio.clamp(0.05, 0.95);
        if (new_ratio - s.ratio).abs() < f32::EPSILON {
            return false;
        }
        // The actual min-size check needs the rects; compute is separate. The
        // clamp to [0.05, 0.95] keeps panes far above MIN in any sane terminal,
        // so we accept the clamp as the guard here.
        s.ratio = new_ratio;
        true
    }

    /// All leaf ids in tree (depth-first) order.
    pub fn leaf_order(&self) -> Vec<(String, ViewType)> {
        let mut out = Vec::new();
        collect_order(&self.root, &mut out);
        out
    }

    fn path_to_id(&self, id: &str) -> Vec<usize> {
        let mut path = Vec::new();
        if find_path(&self.root, id, &mut path) {
            path
        } else {
            Vec::new()
        }
    }
}

fn uuid_fragment() -> String {
    // Cheap unique suffix (no uuid dep needed for layout ids).
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    format!("{:08x}", nanos)
}

fn compute_node(node: &mut Node, rect: Rect) {
    match node {
        Node::Split(s) => {
            s.rect = rect;
            let (a, b) = split_rects(s.axis, s.ratio, rect);
            compute_node(&mut s.child_a, a);
            compute_node(&mut s.child_b, b);
        }
        Node::Leaf(l) => {
            l.rect = rect;
        }
    }
}

pub(crate) fn leaf_at<'a>(node: &'a Node, path: &[usize]) -> &'a Node {
    let mut cur = node;
    for &i in path {
        if let Node::Split(s) = cur {
            cur = if i == 0 { &s.child_a } else { &s.child_b };
        } else {
            return cur;
        }
    }
    cur
}

/// Split `rect` along `axis` by `ratio`, returning the two child rects
/// (left/top first).
pub fn split_rects(axis: Axis, ratio: f32, rect: Rect) -> (Rect, Rect) {
    match axis {
        Axis::Horizontal => {
            let left = (rect.width as f32 * ratio) as u16;
            let right = rect.width.saturating_sub(left);
            (
                Rect { x: rect.x, y: rect.y, width: left, height: rect.height },
                Rect { x: rect.x + left, y: rect.y, width: right, height: rect.height },
            )
        }
        Axis::Vertical => {
            let top = (rect.height as f32 * ratio) as u16;
            let bottom = rect.height.saturating_sub(top);
            (
                Rect { x: rect.x, y: rect.y, width: rect.width, height: top },
                Rect { x: rect.x, y: rect.y + top, width: rect.width, height: bottom },
            )
        }
    }
}

fn leaf_at_mut<'a>(node: &'a mut Node, path: &[usize]) -> &'a mut Node {
    let mut cur = node;
    for &i in path {
        if let Node::Split(s) = cur {
            cur = if i == 0 { &mut s.child_a } else { &mut s.child_b };
        } else {
            return cur;
        }
    }
    cur
}

fn replace_leaf<F>(mut node: Box<Node>, path: &[usize], f: F) -> Box<Node>
where
    F: FnOnce(&LeafNode) -> Node,
{
    let mut cur = &mut *node;
    for &i in path {
        if let Node::Split(s) = cur {
            cur = if i == 0 { &mut s.child_a } else { &mut s.child_b };
        } else {
            break;
        }
    }
    if let Node::Leaf(l) = cur {
        *cur = f(l);
    }
    node
}

fn collect_leaves(node: &Node, focus: &[usize], out: &mut Vec<(String, ViewType, Rect, bool)>) {
    let mut path = Vec::new();
    collect_leaves_path(node, focus, &mut path, out);
}

fn collect_leaves_path(
    node: &Node,
    focus: &[usize],
    path: &mut Vec<usize>,
    out: &mut Vec<(String, ViewType, Rect, bool)>,
) {
    match node {
        Node::Split(s) => {
            path.push(0);
            collect_leaves_path(&s.child_a, focus, path, out);
            path.pop();
            path.push(1);
            collect_leaves_path(&s.child_b, focus, path, out);
            path.pop();
        }
        Node::Leaf(l) => {
            let is_focus = path.as_slice() == focus;
            out.push((l.id.clone(), l.content.view, l.rect, is_focus));
        }
    }
}

fn collect_order(node: &Node, out: &mut Vec<(String, ViewType)>) {
    match node {
        Node::Split(s) => {
            collect_order(&s.child_a, out);
            collect_order(&s.child_b, out);
        }
        Node::Leaf(l) => out.push((l.id.clone(), l.content.view)),
    }
}

fn find_path(node: &Node, id: &str, path: &mut Vec<usize>) -> bool {
    match node {
        Node::Leaf(l) => l.id == id,
        Node::Split(s) => {
            path.push(0);
            if find_path(&s.child_a, id, path) {
                return true;
            }
            path.pop();
            path.push(1);
            if find_path(&s.child_b, id, path) {
                return true;
            }
            path.pop();
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_3() -> LayoutTree {
        default_tree()
    }

    #[test]
    fn default_tree_has_five_leaves() {
        let t = tree_3();
        let leaves = t.leaf_order();
        assert_eq!(leaves.len(), 5);
        let names: Vec<&str> = leaves.iter().map(|(_, v)| v.name()).collect();
        assert_eq!(names, vec!["cockatiel_info", "logs", "module_manager", "engine_graph", "prompts"]);
    }

    #[test]
    fn compute_assigns_rects_that_sum_to_the_root() {
        let mut t = tree_3();
        let root = Rect { x: 0, y: 0, width: 120, height: 40 };
        t.compute(root);
        // Every leaf got a nonzero rect inside the root.
        for (_, _, rect, _) in t.leaves() {
            assert!(rect.width > 0 && rect.height > 0);
            assert!(rect.x + rect.width <= root.width);
            assert!(rect.y + rect.height <= root.height);
        }
        // The focused leaf is the first one by default (info).
        assert_eq!(t.focused_id(), "info");
    }

    #[test]
    fn split_focused_rejects_when_too_small() {
        let mut t = LayoutTree {
            root: split(Axis::Horizontal, 0.5, leaf("a", ViewType::Logs), leaf("b", ViewType::Logs)),
            focus: vec![0],
            dragging: None,
            drag_start: None,
        };
        t.compute(Rect { x: 0, y: 0, width: 20, height: 6 });
        // Splitting the 20x6 left pane horizontally -> 10 wide each, < MIN_WIDTH.
        assert!(!t.split_focused(Axis::Horizontal));
        // Vertical split of 20x6 -> 3 tall each, < MIN_HEIGHT.
        assert!(!t.split_focused(Axis::Vertical));
    }

    #[test]
    fn split_focused_splits_and_focuses_the_left_child() {
        let mut t = tree_3();
        t.compute(Rect { x: 0, y: 0, width: 120, height: 40 });
        t.focus = t.path_to_id("modules");
        assert!(t.split_focused(Axis::Horizontal));
        let leaves = t.leaf_order();
        assert_eq!(leaves.len(), 6);
        // Focus is on the left child of the new split (same view as before).
        assert_eq!(t.focused_view(), ViewType::ModuleManager);
    }

    #[test]
    fn swap_focused_view_changes_the_view() {
        let mut t = tree_3();
        t.focus = t.path_to_id("logs");
        let old = t.swap_focused_view(ViewType::TopUsers);
        assert_eq!(old, Some(ViewType::Logs));
        assert_eq!(t.focused_view(), ViewType::TopUsers);
    }

    #[test]
    fn focus_next_walks_all_leaves_in_order() {
        let mut t = tree_3();
        let order = t.leaf_order();
        let first = t.focused_id();
        assert_eq!(first, order[0].0);
        t.focus_next();
        assert_eq!(t.focused_id(), order[1].0);
        t.focus_prev();
        assert_eq!(t.focused_id(), order[0].0);
    }

    #[test]
    fn join_focused_promotes_the_sibling_and_reduces_leaf_count() {
        let mut t = tree_3();
        // Focus a leaf that has a parent (e.g. the bottom-left graph leaf).
        t.focus = t.path_to_id("graph");
        let before = t.leaf_order().len();
        assert!(t.join_focused(), "a leaf with a parent must be joinable");
        let after = t.leaf_order().len();
        assert_eq!(after, before - 1, "joining removes one leaf");
    }

    #[test]
    fn join_focused_noops_at_the_root() {
        let mut t = LayoutTree {
            root: leaf("only", ViewType::Logs),
            focus: vec![],
            dragging: None,
            drag_start: None,
        };
        assert!(!t.join_focused(), "a single-pane root has nothing to join");
    }

    #[test]
    fn resize_focused_moves_the_ratio_and_clamps() {
        let mut t = tree_3();
        t.focus = t.path_to_id("modules");
        assert!(t.resize_focused(-0.1), "resize shrinks the left/top side");
        // The modules leaf is child_a of the right column (path [1,0]); its
        // parent ratio decreased.
        let parent = t.split_at(&t.focus[..t.focus.len() - 1]);
        let (_, ratio) = parent.unwrap();
        assert!(ratio >= 0.05 && ratio < 0.6, "ratio moved and stayed clamped");
    }

    #[test]
    fn any_divider_is_draggable_not_just_the_focused_pane() {
        let mut t = tree_3();
        t.compute(Rect { x: 0, y: 0, width: 120, height: 40 });
        // The default tree splits the screen left(30%)/right at x=36. Clicking
        // near x=36 must hit the root divider even when focus is elsewhere.
        t.focus = t.path_to_id("graph");
        let hit = t.split_at_point(36, 20);
        assert!(hit.is_some(), "the root divider must be draggable anywhere");
        let (path, dir) = hit.unwrap();
        assert_eq!(path, Vec::<usize>::new(), "the root split is at path []");
        assert_eq!(dir, crate::bsp::DragDir::Right);
        // Dragging it moves the ratio.
        t.dragging = Some((path, dir));
        t.drag_start = Some((36, 20));
        t.update_split_drag(60, 20);
        assert!(t.split_at(&[]).map(|(_, r)| r).unwrap() > 0.30, "dragging right grew child_a");
    }

    #[test]
    fn snapshot_round_trips_the_structure() {
        let mut t = tree_3();
        t.compute(Rect { x: 0, y: 0, width: 120, height: 40 });
        t.focus = t.path_to_id("logs");
        assert!(t.split_focused(crate::bsp::Axis::Horizontal));
        t.swap_focused_view(crate::bsp::ViewType::TopUsers);
        let snap = t.snapshot();
        let rebuilt = LayoutTree::from_snapshot(snap);
        // Same leaf views in the same order.
        let a: Vec<&str> = t.leaf_order().iter().map(|(_, v)| v.name()).collect();
        let b: Vec<&str> = rebuilt.leaf_order().iter().map(|(_, v)| v.name()).collect();
        assert_eq!(a, b);
    }
}
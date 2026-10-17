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
    StreamManager,
    UserManagement,
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
            ViewType::StreamManager,
            ViewType::UserManagement,
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
            ViewType::StreamManager => "stream_manager",
            ViewType::UserManagement => "user_management",
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
            ViewType::StreamManager => WindowId::StreamManager,
            ViewType::UserManagement => WindowId::UserManagement,
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
    /// Returns `None` when any persisted ratio is non-finite — a malformed
    /// snapshot is rejected wholesale rather than half-loaded.
    pub fn from_snapshot(snap: LayoutSnapshot) -> Option<LayoutTree> {
        let root = from_snapshot_node(snap)?;
        // Focus the first leaf (depth-first order) after rebuild.
        let mut tree = LayoutTree { root, focus: Vec::new(), dragging: None, drag_start: None };
        let first = tree.leaf_order().first().cloned().map(|(id, _)| id).unwrap_or_else(|| "pane".to_string());
        tree.focus = tree.path_to_id(&first);
        Some(tree)
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
        LayoutTree::from_snapshot(snap)
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

fn from_snapshot_node(snap: LayoutSnapshot) -> Option<Box<Node>> {
    match snap {
        LayoutSnapshot::Leaf { id, view } => {
            let view = ViewType::from_name(&view).unwrap_or(ViewType::Logs);
            Some(leaf(&id, view))
        }
        LayoutSnapshot::Split { axis, ratio, a, b } => {
            // A persisted ratio of 0.0/1.0 (or any non-finite value) must never
            // survive a reload: clamp it into the legal band so no pane comes
            // back collapsed to zero cells.
            let ratio = sanitize_ratio(ratio)?;
            let axis = if axis == "v" { Axis::Vertical } else { Axis::Horizontal };
            Some(split(axis, ratio, from_snapshot_node(*a)?, from_snapshot_node(*b)?))
        }
    }
}

/// A persisted ratio is legal only inside `0.05..=0.95`. Non-finite values
/// reject the snapshot (`None`); finite out-of-range values are clamped.
fn sanitize_ratio(ratio: f32) -> Option<f32> {
    ratio.is_finite().then(|| ratio.clamp(0.05, 0.95))
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
    // A LOADED view module takes precedence: the module owns the view's core and
    // mints a fresh surface per pane (the TUI loads views the way the engine
    // loads modules). Views not yet migrated to modules build directly below.
    if let Some(m) = crate::cores::registry().module(view.window_id().name()) {
        return m.make_surface();
    }
    use crate::windows::{
        ChartWindow, LogWindow, LogoWindow, ModulesWindow, PromptsWindow, StreamManagerWindow,
        UserManagementWindow, UsersWindow,
    };
    match view {
        ViewType::CockatielInfo => Box::new(LogoWindow),
        ViewType::Logs => Box::new(LogWindow::new()),
        ViewType::ModuleManager => Box::new(ModulesWindow::new()),
        ViewType::EngineGraph => Box::new(ChartWindow::new()),
        ViewType::Prompts => Box::new(PromptsWindow::new()),
        ViewType::TopUsers => Box::new(UsersWindow::new()),
        ViewType::StreamManager => Box::new(StreamManagerWindow::new()),
        ViewType::UserManagement => Box::new(UserManagementWindow::new()),
    }
}

impl LayoutTree {
    /// Recursively compute every leaf's rectangle from `root_rect`. At each
    /// split the stored `ratio` is re-clamped against the split's rect so both
    /// children keep at least `MIN_WIDTH`/`MIN_HEIGHT` cells (or at least one
    /// cell each on a parent too small for the full minimum) — a split can
    /// never produce a zero-cell pane.
    pub fn compute(&mut self, root_rect: Rect) {
        compute_node(&mut self.root, root_rect);
    }

    /// Re-clamp every split against its recomputed rectangle. Called when the
    /// terminal is resized so a divider dragged to the edge at one size cannot
    /// leave a pane collapsed at the new size.
    pub fn renormalize(&mut self, root_rect: Rect) {
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

    /// The focused leaf's window (immutable).
    pub fn focused_window_ref(&self) -> Option<&dyn Window> {
        match leaf_at(&self.root, &self.focus) {
            Node::Leaf(l) => Some(l.content.window.as_ref()),
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

    /// Focus the first leaf hosting `view`. Returns true if such a leaf
    /// existed (focus moved); false if the view is not mounted anywhere.
    pub fn focus_view(&mut self, view: ViewType) -> bool {
        let order = self.leaf_order();
        let id = order.into_iter().find(|(_, v)| *v == view).map(|(id, _)| id);
        match id {
            Some(id) => {
                self.focus = self.path_to_id(&id);
                true
            }
            None => false,
        }
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
    /// split's full extent; positive grows child_a). The candidate ratio is
    /// clamped against the split's own rect so neither child is reduced below
    /// `MIN_WIDTH`/`MIN_HEIGHT` cells (or one cell on a tiny parent).
    pub fn resize_split(&mut self, path: &[usize], delta: f32) -> bool {
        if let Node::Split(s) = leaf_at_mut(&mut self.root, path) {
            let (extent, min_cells) = split_extent_and_min(s);
            let new_ratio = clamp_ratio_to_cells(s.ratio + delta, extent, min_cells);
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
    /// `self.dragging`; the movement is converted to a ratio delta against the
    /// split's OWN extent so the divider tracks the cursor 1:1.
    pub fn update_split_drag(&mut self, x: u16, y: u16) {
        let Some((path, dir)) = self.dragging.clone() else { return };
        let (start_x, start_y) = self.drag_start.unwrap_or((x, y));
        // The ratio is a FRACTION of the split's extent, so moving the cursor by
        // one cell must move the divider by one cell: `delta = pixels / extent`.
        // A fixed divisor (the old `/ 200.0`) made the divider move at a
        // size-dependent fraction of the cursor speed — sluggish on a narrow
        // pane, over-sensitive on a wide one — which is the "drag doesn't follow
        // the mouse" feel. Falls back to the axis only when the split's rect is
        // not yet known (before the first `compute`).
        let Some(extent) = self.split_extent(&path) else { return };
        if extent == 0 {
            return;
        }
        // Dragging a divider right or down ALWAYS grows child_a (the left/top
        // pane), regardless of which side of the divider the cursor grabbed.
        let delta = match dir {
            DragDir::Left | DragDir::Right => {
                (i32::from(x) - i32::from(start_x)) as f32 / extent as f32
            }
            DragDir::Top | DragDir::Bottom => {
                (i32::from(y) - i32::from(start_y)) as f32 / extent as f32
            }
        };
        let _ = self.resize_split(&path, delta);
        self.drag_start = Some((x, y));
    }

    /// The split's extent along its drag axis: width for a left/right split,
    /// height for a top/bottom split. This is the denominator that turns a
    /// cursor delta into a ratio delta. `None` when `path` is not a split.
    fn split_extent(&self, path: &[usize]) -> Option<u16> {
        match leaf_at(&self.root, path) {
            Node::Split(s) => Some(match s.axis {
                Axis::Horizontal => s.rect.width,
                Axis::Vertical => s.rect.height,
            }),
            Node::Leaf(_) => None,
        }
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
    /// left/top child). The candidate ratio is clamped against the parent
    /// split's own rect so the move can never shrink either child below
    /// `MIN_WIDTH`/`MIN_HEIGHT` cells (or one cell on a tiny parent). Returns
    /// false when the clamp leaves the ratio unchanged.
    pub fn resize_focused(&mut self, delta: f32) -> bool {
        if self.focus.len() < 2 {
            return false;
        }
        let parent_path = &self.focus[..self.focus.len() - 1];
        let which = self.focus[self.focus.len() - 1];
        let parent = leaf_at_mut(&mut self.root, parent_path);
        let Node::Split(s) = parent else { return false };

        let candidate = if which == 0 { s.ratio + delta } else { s.ratio - delta };
        let (extent, min_cells) = split_extent_and_min(s);
        let new_ratio = clamp_ratio_to_cells(candidate, extent, min_cells);
        if (new_ratio - s.ratio).abs() < f32::EPSILON {
            return false;
        }
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
            let (extent, min_cells) = split_extent_and_min(s);
            // Re-clamp on every compute so a ratio stored at one terminal size
            // (or loaded from disk) cannot leave a pane collapsed at another.
            s.ratio = clamp_ratio_to_cells(s.ratio, extent, min_cells);
            let (a, b) = split_rects(s.axis, s.ratio, rect);
            compute_node(&mut s.child_a, a);
            compute_node(&mut s.child_b, b);
        }
        Node::Leaf(l) => {
            l.rect = rect;
        }
    }
}

/// The split's extent (in cells) along its axis and the minimum child size that
/// applies to it. `rect` may be unset (`0` extent) before the first `compute`.
fn split_extent_and_min(s: &SplitNode) -> (u16, u16) {
    match s.axis {
        Axis::Horizontal => (s.rect.width, MIN_WIDTH),
        Axis::Vertical => (s.rect.height, MIN_HEIGHT),
    }
}

/// Clamp `ratio` so that neither child of a split is collapsed to zero cells.
/// Given the parent `extent` in cells and the applicable `min_cells`, each side
/// keeps at least `min_cells` when the parent is large enough; on a parent too
/// small for that, the floor degrades to half the extent so both children still
/// get at least one cell whenever `extent >= 2`. The result is always
/// intersected with the absolute `0.05..=0.95` band. An unset (`< 2`) extent
/// falls back to that band.
fn clamp_ratio_to_cells(ratio: f32, extent: u16, min_cells: u16) -> f32 {
    if !ratio.is_finite() {
        return 0.5;
    }
    if extent < 2 {
        return ratio.clamp(0.05, 0.95);
    }
    // Half a cell of slack absorbs the float→cell truncation in `split_rects`,
    // so a ratio at the floor still yields `min_cells` whole cells.
    let floor = (min_cells as f32 + 0.5).min(extent as f32 / 2.0) / extent as f32;
    let lo = floor.max(0.05);
    let hi = (1.0 - floor).min(0.95);
    ratio.clamp(lo, hi)
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
            let left = split_offset(rect.width, ratio);
            let right = rect.width.saturating_sub(left);
            (
                Rect { x: rect.x, y: rect.y, width: left, height: rect.height },
                Rect { x: rect.x + left, y: rect.y, width: right, height: rect.height },
            )
        }
        Axis::Vertical => {
            let top = split_offset(rect.height, ratio);
            let bottom = rect.height.saturating_sub(top);
            (
                Rect { x: rect.x, y: rect.y, width: rect.width, height: top },
                Rect { x: rect.x, y: rect.y + top, width: rect.width, height: bottom },
            )
        }
    }
}

/// The child_a extent (in cells) of a split of `extent` cells by `ratio`. Keeps
/// at least one cell on each side whenever `extent >= 2`, so a divider can never
/// collapse a pane to zero. `ratio` is clamped to `0..=1` first so a malformed
/// value cannot overflow the cast.
fn split_offset(extent: u16, ratio: f32) -> u16 {
    let ratio = if ratio.is_finite() { ratio.clamp(0.0, 1.0) } else { 0.5 };
    let raw = (extent as f32 * ratio) as u16;
    if extent >= 2 {
        raw.clamp(1, extent - 1)
    } else {
        raw.min(extent)
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
    fn a_divider_drag_tracks_the_cursor_one_to_one() {
        // Dragging a divider by N cells must move it by exactly N cells, on any
        // terminal size. The old fixed `/ 200.0` divisor moved it by
        // `extent * N / 200` — sluggish on a narrow pane, over-sensitive on a
        // wide one — which is the "the drag doesn't follow the mouse" feel.
        for size in [40u16, 120, 300] {
            // Root is a HORIZONTAL (left/right) split at ratio 0.30.
            let mut t = tree_3();
            t.compute(Rect { x: 0, y: 0, width: size, height: 40 });
            let root = Vec::<usize>::new();
            let (_, ratio0) = t.split_at(&root).unwrap();
            let start = (ratio0 * size as f32) as u16;
            let target = start + size / 4;
            t.dragging = Some((root.clone(), DragDir::Right));
            t.drag_start = Some((start, 20));
            t.update_split_drag(target, 20);
            let (_, ratio1) = t.split_at(&root).unwrap();
            let landed = (ratio1 * size as f32).round() as i32;
            assert!(
                (landed - i32::from(target)).abs() <= 1,
                "h size={size}: divider landed at {landed}, cursor at {target}"
            );

            // The left column is a VERTICAL (top/bottom) split at ratio 0.50.
            let mut t = tree_3();
            t.compute(Rect { x: 0, y: 0, width: size, height: 40 });
            let left = vec![0usize];
            let (_, ratio0) = t.split_at(&left).unwrap();
            let start = (ratio0 * 40.0) as u16;
            let target = start + 10;
            t.dragging = Some((left.clone(), DragDir::Bottom));
            t.drag_start = Some((5, start));
            t.update_split_drag(5, target);
            let (_, ratio1) = t.split_at(&left).unwrap();
            let landed = (ratio1 * 40.0).round() as i32;
            assert!(
                (landed - i32::from(target)).abs() <= 1,
                "v size={size}: divider landed at {landed}, cursor at {target}"
            );
        }
    }

    #[test]
    fn snapshot_round_trips_the_structure() {
        let mut t = tree_3();
        t.compute(Rect { x: 0, y: 0, width: 120, height: 40 });
        t.focus = t.path_to_id("logs");
        assert!(t.split_focused(crate::bsp::Axis::Horizontal));
        t.swap_focused_view(crate::bsp::ViewType::TopUsers);
        let snap = t.snapshot();
        let rebuilt = LayoutTree::from_snapshot(snap).expect("a well-formed snapshot loads");
        // Same leaf views in the same order.
        let a: Vec<&str> = t.leaf_order().iter().map(|(_, v)| v.name()).collect();
        let b: Vec<&str> = rebuilt.leaf_order().iter().map(|(_, v)| v.name()).collect();
        assert_eq!(a, b);
    }

    /// A two-leaf tree split along `axis`, focused on child_a.
    fn two_leaf(axis: Axis) -> LayoutTree {
        LayoutTree {
            root: split(axis, 0.5, leaf("a", ViewType::Logs), leaf("b", ViewType::Logs)),
            focus: vec![0],
            dragging: None,
            drag_start: None,
        }
    }

    #[test]
    fn resizing_to_an_extreme_keeps_both_children_at_least_min() {
        let root = Rect { x: 0, y: 0, width: 120, height: 40 };

        // Dragging the divider hard right must stop so child_b keeps MIN_WIDTH.
        let mut t = two_leaf(Axis::Horizontal);
        t.compute(root);
        assert!(t.resize_split(&[], 10.0), "a big right drag must move the divider");
        t.compute(root);
        let (_, ratio) = t.split_at(&[]).unwrap();
        let (a, b) = split_rects(Axis::Horizontal, ratio, root);
        assert!(a.width >= MIN_WIDTH && b.width >= MIN_WIDTH, "right drag: a={} b={}", a.width, b.width);

        // Dragging hard left must stop so child_a keeps MIN_WIDTH.
        let mut t = two_leaf(Axis::Horizontal);
        t.compute(root);
        assert!(t.resize_split(&[], -10.0), "a big left drag must move the divider");
        t.compute(root);
        let (_, ratio) = t.split_at(&[]).unwrap();
        let (a, b) = split_rects(Axis::Horizontal, ratio, root);
        assert!(a.width >= MIN_WIDTH && b.width >= MIN_WIDTH, "left drag: a={} b={}", a.width, b.width);

        // The focused-resize path enforces the same cell-aware floor. It acts on
        // the focused leaf's PARENT split, so the leaf must sit below the root.
        let mut t = LayoutTree {
            root: split(
                Axis::Horizontal,
                0.5,
                split(Axis::Vertical, 0.5, leaf("a", ViewType::Logs), leaf("b", ViewType::Logs)),
                leaf("c", ViewType::Logs),
            ),
            focus: vec![0, 0],
            dragging: None,
            drag_start: None,
        };
        t.compute(root);
        assert!(t.resize_focused(10.0), "a big focused resize must move the divider");
        t.compute(root);
        let (_, ratio) = t.split_at(&[0]).unwrap();
        let (a, b) = split_rects(Axis::Vertical, ratio, Rect { x: 0, y: 0, width: 60, height: 40 });
        assert!(a.height >= MIN_HEIGHT && b.height >= MIN_HEIGHT, "focused: a={} b={}", a.height, b.height);
    }

    #[test]
    fn a_snapshot_with_collapsed_ratios_loads_without_zero_cell_panes() {
        // 0.0 and 1.0 are exactly the ratios a divider dragged to the edge
        // persists; both must be clamped on load.
        let snap = LayoutSnapshot::Split {
            axis: "h".to_string(),
            ratio: 0.0,
            a: Box::new(LayoutSnapshot::Leaf { id: "a".to_string(), view: "logs".to_string() }),
            b: Box::new(LayoutSnapshot::Split {
                axis: "v".to_string(),
                ratio: 1.0,
                a: Box::new(LayoutSnapshot::Leaf { id: "b".to_string(), view: "logs".to_string() }),
                b: Box::new(LayoutSnapshot::Leaf { id: "c".to_string(), view: "prompts".to_string() }),
            }),
        };
        let mut t = LayoutTree::from_snapshot(snap).expect("out-of-range ratios are clamped, not rejected");
        t.compute(Rect { x: 0, y: 0, width: 120, height: 40 });
        for (id, _, rect, _) in t.leaves() {
            assert!(rect.width > 0 && rect.height > 0, "pane {id} collapsed: {rect:?}");
        }
    }

    #[test]
    fn a_non_finite_ratio_rejects_the_whole_snapshot() {
        let snap = LayoutSnapshot::Split {
            axis: "h".to_string(),
            ratio: f32::NAN,
            a: Box::new(LayoutSnapshot::Leaf { id: "a".to_string(), view: "logs".to_string() }),
            b: Box::new(LayoutSnapshot::Leaf { id: "b".to_string(), view: "logs".to_string() }),
        };
        assert!(LayoutTree::from_snapshot(snap).is_none());
    }

    #[test]
    fn a_tiny_parent_extent_never_zeroes_a_child() {
        for extent in 2u16..10 {
            let rect = Rect { x: 0, y: 0, width: extent, height: extent };
            let mut t = two_leaf(Axis::Horizontal);
            t.compute(rect);
            let (_, ratio) = t.split_at(&[]).unwrap();
            let (a, b) = split_rects(Axis::Horizontal, ratio, rect);
            assert!(a.width >= 1 && b.width >= 1, "extent={extent}: a={} b={}", a.width, b.width);

            let mut t = two_leaf(Axis::Vertical);
            t.compute(rect);
            let (_, ratio) = t.split_at(&[]).unwrap();
            let (a, b) = split_rects(Axis::Vertical, ratio, rect);
            assert!(a.height >= 1 && b.height >= 1, "extent={extent}: a={} b={}", a.height, b.height);
        }
        // extent < 2 cannot give both children a cell, but must not panic.
        let mut t = two_leaf(Axis::Horizontal);
        t.compute(Rect { x: 0, y: 0, width: 1, height: 1 });
    }

    #[test]
    fn focus_view_finds_a_mounted_view_and_never_splits() {
        let mut t = tree_3();
        // Default tree has no top_users pane.
        assert!(!t.focus_view(crate::bsp::ViewType::TopUsers));
        // Mount one by swapping the logs leaf, then it is focusable.
        t.swap_view_by_id("logs", crate::bsp::ViewType::TopUsers);
        let leaves_before = t.leaf_order().len();
        assert!(t.focus_view(crate::bsp::ViewType::TopUsers));
        assert_eq!(t.focused_view(), crate::bsp::ViewType::TopUsers);
        // Focusing a view must never add panes (no auto-split).
        assert_eq!(t.leaf_order().len(), leaves_before);
    }
}
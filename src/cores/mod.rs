//! Per-view **cores**: the source of truth for a view type.
//!
//! A core owns a view's DATA and its PRESENTATION logic, and runs for the life
//! of the process — the same shape as the engine being the source of truth for
//! modules. Every visible subview (an embedded pane or a detached pop-out) is a
//! thin surface that hooks into a core: it holds only its own interaction state
//! (cursor/scroll/filter) and asks the core to draw into its area.
//!
//! This is the migration target away from the monolithic `GlobalStats`/globals;
//! `LogCore` is the first core, and the rest follow one at a time.

pub mod log;

use std::sync::OnceLock;

use crate::app::Window;

/// A **view module**: a self-contained unit that owns a view's data (its core)
/// AND its presentation logic, registered so the TUI can LOAD it and let the
/// operator lay it out however they wish (panes, pop-outs). The same shape as
/// the engine loading modules — but for views inside the TUI.
///
/// A module is long-lived: it holds the core (the source of truth) for the life
/// of the process. Each pane/pop-out that shows the view gets a fresh
/// **surface** from [`ViewModule::make_surface`], which holds only its own
/// interaction state (cursor/scroll/filter) and hooks into the module's core.
pub trait ViewModule: Send + Sync {
    /// Stable id, matching the persisted layout and the `--detached <id>`
    /// argument (e.g. "log").
    fn id(&self) -> &'static str;
    /// Human title for the pane header / view list.
    #[allow(dead_code)]
    fn title(&self) -> &'static str;
    /// Mint a fresh surface (per-pane UI) over this module's core.
    fn make_surface(&self) -> Box<dyn Window>;
}

/// The set of loaded view modules, in presentation order.
pub struct ViewRegistry {
    modules: Vec<Box<dyn ViewModule>>,
}

impl ViewRegistry {
    /// The module with `id`, if it is loaded.
    pub fn module(&self, id: &str) -> Option<&dyn ViewModule> {
        self.modules.iter().find(|m| m.id() == id).map(|m| m.as_ref())
    }

    /// Every loaded module id, in order.
    #[allow(dead_code)]
    pub fn ids(&self) -> Vec<&'static str> {
        self.modules.iter().map(|m| m.id()).collect()
    }
}

/// Build the registry from every view module the TUI ships. Adding a view
/// becomes "write the module + register it here" — no changes to the tree.
fn build_registry() -> ViewRegistry {
    ViewRegistry {
        modules: vec![Box::new(log::LogModule)],
    }
}

static REGISTRY: OnceLock<ViewRegistry> = OnceLock::new();

/// The process-wide loaded view modules.
pub fn registry() -> &'static ViewRegistry {
    REGISTRY.get_or_init(build_registry)
}

//! Runtime path resolution: where the engine, user database, modules and TUI
//! config live.
//!
//! The TUI used to bake every sibling path in at compile time via
//! `env!("CARGO_MANIFEST_DIR")`, which only works from a monorepo checkout. A
//! self-contained install (a single root holding `bin/`, `engine/`, `user-db/`,
//! `modules/` and `config/`) needs the same paths resolved at RUNTIME so the
//! binaries can be relocated as a unit.
//!
//! Precedence: an explicit `--install-root` / `COCKATIEL_HOME`, then a root
//! detected from the running executable (`<root>/bin/<exe>` next to
//! `<root>/engine`), then the legacy monorepo layout — which is byte-for-byte
//! the old behavior so development is unchanged.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Which on-disk arrangement the resolved [`Paths`] describes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// A self-contained install root: `bin/`, `engine/`, `user-db/`,
    /// `modules/`, `config/`, `rank_chart.json`.
    Installed,
    /// The monorepo checkout the TUI has always run from.
    Legacy,
}

/// Every filesystem root the supervisor needs, resolved once at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Kept as part of the descriptor for diagnostics/tests; the runtime reads
    /// the resolved fields below.
    #[allow(dead_code)]
    pub layout: Layout,
    /// For an installed layout, the install root; for legacy, the TUI crate
    /// dir (`CARGO_MANIFEST_DIR`). Kept for diagnostics/tests.
    #[allow(dead_code)]
    pub root: PathBuf,
    pub engine_dir: PathBuf,
    pub user_db_dir: PathBuf,
    pub tui_dir: PathBuf,
    pub modules_dir: PathBuf,
    pub rank_chart: PathBuf,
}

/// Resolve the layout from explicit inputs only — no environment reads, and no
/// filesystem access beyond the `engine/` existence probe rule 2 requires.
/// Pure so the precedence is pinned by unit tests instead of being observable
/// only by running an installed build.
pub fn resolve(install_root: Option<&Path>, exe: Option<&Path>, legacy_root: &Path) -> Paths {
    // 1. An explicit root always wins.
    if let Some(root) = install_root {
        return installed(root);
    }
    // 2. An exe under `<root>/bin` beside `<root>/engine` proves an install.
    if let Some(root) = exe.and_then(installed_root_from_exe) {
        return installed(&root);
    }
    // 3. Otherwise the monorepo checkout, exactly as before.
    legacy(legacy_root)
}

/// The installed layout: everything hangs off a single relocatable root.
fn installed(root: &Path) -> Paths {
    Paths {
        layout: Layout::Installed,
        root: root.to_path_buf(),
        engine_dir: root.join("engine"),
        user_db_dir: root.join("user-db"),
        tui_dir: root.join("config"),
        modules_dir: root.join("modules"),
        rank_chart: root.join("rank_chart.json"),
    }
}

/// The legacy monorepo layout: the sibling crates and repo-root chart.
fn legacy(root: &Path) -> Paths {
    let parent = root.join("..");
    Paths {
        layout: Layout::Legacy,
        root: root.to_path_buf(),
        engine_dir: parent.join("cockatiel_engine-rs"),
        user_db_dir: parent.join("cockatiel_user_database-rs"),
        tui_dir: root.to_path_buf(),
        modules_dir: parent.join("modules"),
        rank_chart: parent.join("rank_chart.json"),
    }
}

/// Detect an install root from the running executable: an exe in `<root>/bin`
/// whose sibling `<root>/engine` exists is a self-contained install. The
/// `engine/` check is what distinguishes a real install from an unrelated
/// `bin/` directory that merely happens to contain the TUI.
fn installed_root_from_exe(exe: &Path) -> Option<PathBuf> {
    let parent = exe.parent()?;
    if parent.file_name() != Some(OsStr::new("bin")) {
        return None;
    }
    let root = parent.parent()?;
    if root.join("engine").exists() {
        Some(root.to_path_buf())
    } else {
        None
    }
}

static PATHS: OnceLock<Paths> = OnceLock::new();
static MODULES_OVERRIDE: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Resolve and cache the layout for this process. Called once from `main`
/// immediately after CLI parsing, BEFORE any path accessor. The explicit
/// `install_root` wins over `COCKATIEL_HOME`; both win over exe detection and
/// the legacy fallback. `modules_override` is the `--modules-dir` escape hatch.
pub fn init(install_root: Option<PathBuf>, modules_override: Option<PathBuf>) {
    let root = install_root.or_else(|| std::env::var_os("COCKATIEL_HOME").map(PathBuf::from));
    let exe = std::env::current_exe().ok();
    let _ = PATHS.set(resolve(
        root.as_deref(),
        exe.as_deref(),
        Path::new(env!("CARGO_MANIFEST_DIR")),
    ));
    let _ = MODULES_OVERRIDE.set(modules_override);
}

/// The resolved layout. Resolved lazily on first use when `init` was never
/// called, so tests and directly-started detached windows still work.
pub fn current() -> &'static Paths {
    PATHS.get_or_init(|| {
        let root = std::env::var_os("COCKATIEL_HOME").map(PathBuf::from);
        let exe = std::env::current_exe().ok();
        resolve(
            root.as_deref(),
            exe.as_deref(),
            Path::new(env!("CARGO_MANIFEST_DIR")),
        )
    })
}

/// The modules directory: the `--modules-dir` override when set, else the
/// layout's default.
pub fn modules_dir() -> PathBuf {
    if let Some(Some(dir)) = MODULES_OVERRIDE.get() {
        return dir.clone();
    }
    current().modules_dir.clone()
}

/// Candidate locations for a service binary inside `dir`, in preference order:
/// the installed flat layout first, then the legacy cargo target dirs. The
/// `.exe` suffix is added on Windows. Pure, so callers (and tests) can build
/// the exact same list for error messages.
pub fn binary_candidates(dir: &Path, name: &str) -> Vec<PathBuf> {
    let name = if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    vec![
        dir.join(&name),
        dir.join("target").join("release").join(&name),
        dir.join("target").join("debug").join(&name),
    ]
}

fn binary_in(dir: &Path, name: &str) -> Option<PathBuf> {
    binary_candidates(dir, name).into_iter().find(|p| p.exists())
}

/// The engine executable: installed flat layout first, else the legacy target
/// dirs. `None` when nothing exists.
pub fn engine_binary() -> Option<PathBuf> {
    binary_in(&current().engine_dir, "cockatiel-engine-rs")
}

/// The user-database executable: installed flat layout first, else the legacy
/// target dirs. `None` when nothing exists.
pub fn user_db_binary() -> Option<PathBuf> {
    binary_in(&current().user_db_dir, "cockatiel-user-database")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("{tag}-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn explicit_install_root_uses_the_installed_layout() {
        let root = Path::new("/opt/cockatiel");
        let p = resolve(Some(root), None, Path::new("/monorepo/cockatiel_tui_v2-rs"));
        assert_eq!(p.layout, Layout::Installed);
        assert_eq!(p.root.as_path(), root);
        assert_eq!(p.engine_dir, root.join("engine"));
        assert_eq!(p.user_db_dir, root.join("user-db"));
        assert_eq!(p.tui_dir, root.join("config"));
        assert_eq!(p.modules_dir, root.join("modules"));
        assert_eq!(p.rank_chart, root.join("rank_chart.json"));
    }

    #[test]
    fn exe_under_bin_next_to_engine_detects_the_install_root() {
        let root = temp_root("ck-paths-exe");
        std::fs::create_dir_all(root.join("engine")).unwrap();
        let exe = root.join("bin").join("cockatiel-tui-v2");
        let p = resolve(None, Some(&exe), Path::new("/monorepo/cockatiel_tui_v2-rs"));
        assert_eq!(p.layout, Layout::Installed);
        assert_eq!(p.root, root);
        assert_eq!(p.engine_dir, root.join("engine"));
        assert_eq!(p.tui_dir, root.join("config"));
        assert_eq!(p.modules_dir, root.join("modules"));
        assert_eq!(p.rank_chart, root.join("rank_chart.json"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn exe_under_bin_without_engine_dir_falls_back_to_legacy() {
        // A `bin/` parent alone is not proof of an install; the `engine/`
        // sibling is. Without it the monorepo layout must win.
        let root = temp_root("ck-paths-noeng");
        let exe = root.join("bin").join("cockatiel-tui-v2");
        let legacy = Path::new("/monorepo/cockatiel_tui_v2-rs");
        let p = resolve(None, Some(&exe), legacy);
        assert_eq!(p.layout, Layout::Legacy);
        assert_eq!(p.engine_dir, legacy.join("..").join("cockatiel_engine-rs"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_layout_matches_the_compile_time_paths() {
        let legacy = Path::new("/monorepo/cockatiel_tui_v2-rs");
        let p = resolve(None, None, legacy);
        assert_eq!(p.layout, Layout::Legacy);
        assert_eq!(p.engine_dir, legacy.join("..").join("cockatiel_engine-rs"));
        assert_eq!(p.user_db_dir, legacy.join("..").join("cockatiel_user_database-rs"));
        assert_eq!(p.tui_dir, legacy);
        assert_eq!(p.modules_dir, legacy.join("..").join("modules"));
        assert_eq!(p.rank_chart, legacy.join("..").join("rank_chart.json"));
    }

    #[test]
    fn engine_binary_prefers_installed_then_release_then_debug() {
        // Exercises the same resolver `engine_binary()` uses, with a temp
        // `<dir>/engine` so the ordering is pinned without touching the
        // process-global OnceLock.
        let root = temp_root("ck-paths-bin");
        let engine = root.join("engine");
        std::fs::create_dir_all(&engine).unwrap();
        let cands = binary_candidates(&engine, "cockatiel-engine-rs");
        assert_eq!(cands.len(), 3);
        let (flat, release, debug) = (cands[0].clone(), cands[1].clone(), cands[2].clone());

        // Nothing present -> None.
        assert_eq!(binary_in(&engine, "cockatiel-engine-rs"), None);

        // Legacy debug only.
        std::fs::create_dir_all(debug.parent().unwrap()).unwrap();
        std::fs::write(&debug, b"x").unwrap();
        assert_eq!(binary_in(&engine, "cockatiel-engine-rs"), Some(debug.clone()));

        // Legacy release beats debug.
        std::fs::create_dir_all(release.parent().unwrap()).unwrap();
        std::fs::write(&release, b"x").unwrap();
        assert_eq!(binary_in(&engine, "cockatiel-engine-rs"), Some(release.clone()));

        // The installed flat layout beats both.
        std::fs::write(&flat, b"x").unwrap();
        assert_eq!(binary_in(&engine, "cockatiel-engine-rs"), Some(flat));

        let _ = std::fs::remove_dir_all(&root);
    }
}

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::app::{Window};
use crate::colors::ColorConfig;
use crate::cores::log::{log_core, LogView};
use crate::db::GlobalStats;
use crate::hotkeys::Action;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct LogEntry {
    #[allow(dead_code)]
    pub timestamp: String,
    pub source: String,
    pub message: String,
    pub event_type: i32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LogFilter {
    All,
    Logs,
    Errors,
    System,
    Messages,
}

impl LogFilter {
    pub fn label(&self) -> &'static str {
        match self {
            LogFilter::All => "All",
            LogFilter::Logs => "Logs",
            LogFilter::Errors => "Errors",
            LogFilter::System => "System",
            LogFilter::Messages => "Messages",
        }
    }

    pub fn all() -> &'static [LogFilter] {
        &[LogFilter::All, LogFilter::Logs, LogFilter::Errors, LogFilter::System, LogFilter::Messages]
    }

    pub(crate) fn matches(&self, entry: &LogEntry) -> bool {
        match self {
            LogFilter::All => true,
            LogFilter::Logs => entry.event_type == 1,
            LogFilter::Errors => entry.event_type == 3,
            LogFilter::System => entry.event_type == 4,
            LogFilter::Messages => entry.event_type == 5,
        }
    }
}

/// The log view: a THIN surface over the shared [`crate::cores::log::LogCore`].
/// It holds only its own filter/scroll ([`LogView`]); the data and the drawing
/// live in the core, so every log pane (and every pop-out) shows the same lines
/// with its own cursor.
pub struct LogWindow {
    view: LogView,
}

impl LogWindow {
    pub fn new() -> Self {
        Self {
            view: LogView::default(),
        }
    }
}

impl Window for LogWindow {
    fn push_log(&mut self, entry: LogEntry) {
        // Kept as the window-facing entry point (tests use it); the data lives in
        // the core.
        log_core().push(entry);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer, is_active: bool, _stats: &GlobalStats, colors: &ColorConfig, _hotkeys: &crate::hotkeys::HotkeyConfig, _prompts: &[crate::app::PendingPrompt]) {
        // Delegate the presentation logic to the core.
        log_core().render(area, buf, self.view, is_active, colors);
    }

    fn handle_key(&mut self, key: crossterm::event::KeyEvent, _stats: &mut GlobalStats) -> Option<Action> {
        match key.code {
            crossterm::event::KeyCode::Char('1') => { self.view.filter = LogFilter::All; Some(Action::Noop) }
            crossterm::event::KeyCode::Char('2') => { self.view.filter = LogFilter::Logs; Some(Action::Noop) }
            crossterm::event::KeyCode::Char('3') => { self.view.filter = LogFilter::Errors; Some(Action::Noop) }
            crossterm::event::KeyCode::Char('4') => { self.view.filter = LogFilter::System; Some(Action::Noop) }
            crossterm::event::KeyCode::Char('5') => { self.view.filter = LogFilter::Messages; Some(Action::Noop) }
            crossterm::event::KeyCode::Char('j') | crossterm::event::KeyCode::Down => {
                self.view.scroll = self.view.scroll.saturating_add(1);
                Some(Action::Noop)
            }
            crossterm::event::KeyCode::Char('k') | crossterm::event::KeyCode::Up => {
                self.view.scroll = self.view.scroll.saturating_sub(1);
                Some(Action::Noop)
            }
            crossterm::event::KeyCode::PageDown => {
                self.view.scroll = self.view.scroll.saturating_add(10);
                Some(Action::Noop)
            }
            crossterm::event::KeyCode::PageUp => {
                self.view.scroll = self.view.scroll.saturating_sub(10);
                Some(Action::Noop)
            }
            crossterm::event::KeyCode::Char('w') => Some(Action::PopOut("log".to_string())),
            _ => None,
        }
    }
}

#[cfg(test)]
mod log_bounds_tests {
    use super::*;
    use crate::hotkeys::default_hotkeys;
    use crate::layout::LayoutState;
    use ratatui::buffer::Buffer;

    fn painted(buf: &Buffer) -> Vec<(u16, u16)> {
        let area = buf.area;
        let mut out = Vec::new();
        for y in area.y..area.y + area.height {
            for x in area.x..area.x + area.width {
                if buf[(x, y)].symbol() != " " {
                    out.push((x, y));
                }
            }
        }
        out
    }

    fn probe(term_w: u16, term_h: u16, pct: u16) {
        let terminal = Rect { x: 0, y: 0, width: term_w, height: term_h };
        let ls = LayoutState {
            left_width_pct: pct,
            ..LayoutState::default()
        };
        let areas = ls.compute(terminal);

        let mut w = LogWindow::new();
        for i in 0..40 {
            w.push_log(LogEntry {
                timestamp: String::new(),
                source: format!("discord-adapter-{}", i),
                message: format!("a fairly long log message number {} that goes on a bit", i),
                event_type: 1 + (i % 5),
            });
        }

        let mut buf = Buffer::empty(terminal);
        let colors = crate::colors::load_colors(&std::path::PathBuf::from(""));
        w.render(areas.log, &mut buf, true, &crate::db::GlobalStats::default(), &colors, &default_hotkeys(), &[]);

        let outside: Vec<(u16, u16)> = painted(&buf).into_iter()
            .filter(|(x, y)| {
                !(*x >= areas.log.x && *x < areas.log.x + areas.log.width
                  && *y >= areas.log.y && *y < areas.log.y + areas.log.height)
            })
            .collect();
        assert!(
            outside.is_empty(),
            "log wrote {} cell(s) outside its area {:?} (modules is {:?}) -- first: {:?}",
            outside.len(),
            areas.log,
            areas.modules,
            outside.iter().take(6).collect::<Vec<_>>()
        );
    }

    /// The reported "log overflows into modules" symptom. The log window is
    /// provably confined to its own rect for every legal layout, so anything
    /// seen bleeding across is a stale-cell artifact of the diff-based
    /// renderer rather than a bounds bug -- which is what the full-repaint on
    /// a shape change fixes.
    #[test]
    fn log_never_paints_outside_its_own_area() {
        let _g = crate::cores::log::LOG_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        for (w, h) in [(120u16, 40u16), (200, 60), (80, 24), (60, 20)] {
            for pct in [10u16, 30, 50, 90] {
                probe(w, h, pct);
            }
        }
    }
}

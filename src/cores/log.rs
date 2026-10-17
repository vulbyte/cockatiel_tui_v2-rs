//! The log core: owns every log line and the logic that draws the log view.
//!
//! Before this, a `LogWindow` owned its own `VecDeque` (so a second pane was
//! empty) and the rendering lived in the window. Now the core owns the data and
//! the drawing; a window keeps only its filter/scroll ([`LogView`]) and asks the
//! core to render into its area. A detached pop-out hooks into its own mirror of
//! this core, so it draws identically at any size.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::colors::ColorConfig;
use crate::windows::log::{LogEntry, LogFilter};

/// How many lines the core keeps (oldest dropped first).
const LOG_CAP: usize = 500;

/// The per-surface interaction state passed INTO the core's render. The DATA is
/// the core's; the filter/scroll belong to the surface, so two panes over one
/// core keep independent views.
#[derive(Clone, Copy)]
pub struct LogView {
    pub filter: LogFilter,
    pub scroll: usize,
}

impl Default for LogView {
    fn default() -> Self {
        Self {
            filter: LogFilter::All,
            scroll: 0,
        }
    }
}

/// The log view's source of truth.
pub struct LogCore {
    lines: Mutex<VecDeque<LogEntry>>,
    /// Bumped on every change, so a mirror (a detached window) can tell when it
    /// is stale without diffing the whole buffer.
    rev: AtomicUsize,
}

impl LogCore {
    fn new() -> Self {
        Self {
            lines: Mutex::new(VecDeque::new()),
            rev: AtomicUsize::new(0),
        }
    }

    /// Append one line (dropping the oldest past the cap).
    pub fn push(&self, entry: LogEntry) {
        let mut q = self.lines.lock().unwrap();
        q.push_back(entry);
        while q.len() > LOG_CAP {
            q.pop_front();
        }
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// Replace every line (a detached window adopting its parent's snapshot).
    pub fn set(&self, entries: Vec<LogEntry>) {
        let mut q = self.lines.lock().unwrap();
        q.clear();
        q.extend(entries);
        self.rev.fetch_add(1, Ordering::Relaxed);
    }

    /// A copy of every line, oldest first.
    pub fn snapshot(&self) -> Vec<LogEntry> {
        self.lines.lock().unwrap().iter().cloned().collect()
    }

    /// This surface's lines, filtered by its own filter.
    fn filtered(&self, filter: LogFilter) -> Vec<LogEntry> {
        self.lines
            .lock()
            .unwrap()
            .iter()
            .filter(|e| filter.matches(e))
            .cloned()
            .collect()
    }

    fn tab_bar(filter: LogFilter) -> Line<'static> {
        let mut spans = vec![Span::styled("  ", Style::default())];
        for f in LogFilter::all().iter() {
            let style = if *f == filter {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            spans.push(Span::styled(format!("[{}] ", f.label()), style));
        }
        Line::from(spans)
    }

    /// The core's presentation logic: draw the log view into `area` using the
    /// surface's own `view` (filter/scroll). This is what every log surface —
    /// an embedded pane or a pop-out — calls.
    pub fn render(
        &self,
        area: Rect,
        buf: &mut Buffer,
        view: LogView,
        is_active: bool,
        colors: &ColorConfig,
    ) {
        let border_color = if is_active {
            colors.active_border_color("log")
        } else {
            colors.border_color("inactive")
        };

        let block = Block::default()
            .title(" log ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner = area.inner(ratatui::layout::Margin {
            horizontal: 1,
            vertical: 1,
        });

        let key_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);
        let desc_style = Style::default().fg(Color::DarkGray);
        let hotkey = crate::windows::hotkey_wrap::layout(
            &[
                ("1-5".to_string(), key_style),
                (":filter".to_string(), desc_style),
                ("j/k".to_string(), key_style),
                (":scroll".to_string(), desc_style),
                (
                    "w".to_string(),
                    Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                ),
            ],
            area,
            inner,
        );
        let inner = hotkey.content;

        let tab_area = Rect {
            x: inner.x,
            y: inner.y,
            width: inner.width,
            height: 1,
        };
        Paragraph::new(Self::tab_bar(view.filter)).render(tab_area, buf);

        let log_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1),
        };

        let filtered = self.filtered(view.filter);
        let mut lines: Vec<Line> = Vec::new();

        let visible = log_area.height as usize;
        let total = filtered.len();
        let scroll = view.scroll.min(total.saturating_sub(1));
        let start = total.saturating_sub(scroll + visible);
        for entry in filtered.iter().skip(start).take(visible) {
            let type_color = match entry.event_type {
                1 => Color::DarkGray,
                2 => Color::Yellow,
                3 => Color::Red,
                4 => Color::Cyan,
                5 => Color::Green,
                _ => Color::DarkGray,
            };
            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", entry.source), Style::default().fg(type_color)),
                Span::styled(&entry.message, Style::default().fg(Color::Gray)),
            ]));
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (no entries)",
                Style::default().fg(Color::DarkGray),
            )));
        }
        Paragraph::new(lines).render(log_area, buf);

        block.render(area, buf);
        Paragraph::new(hotkey.lines).render(hotkey.area, buf);
    }
}

static LOG_CORE: OnceLock<Arc<LogCore>> = OnceLock::new();

/// Serializes tests that read/write the process-wide log core, so one test's
/// push flood cannot evict another's lines mid-assertion (the cap is 500).
#[cfg(test)]
pub static LOG_TEST_LOCK: Mutex<()> = Mutex::new(());

/// The process-wide log core. Sources with no `AppState` handle (the supervisor
/// log, module stdout) push straight into it; a window only renders it.
pub fn log_core() -> Arc<LogCore> {
    LOG_CORE.get_or_init(|| Arc::new(LogCore::new())).clone()
}

/// The log **view module**: registered so the TUI can load it and mint log
/// surfaces. Its core ([`log_core`]) is the module's source of truth.
pub struct LogModule;

impl crate::cores::ViewModule for LogModule {
    fn id(&self) -> &'static str {
        "log"
    }

    fn title(&self) -> &'static str {
        "log"
    }

    fn make_surface(&self) -> Box<dyn crate::app::Window> {
        Box::new(crate::windows::log::LogWindow::new())
    }
}

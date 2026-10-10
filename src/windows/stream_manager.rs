//! Stream manager: schedule a stream (title, start time, thumbnail) while the
//! channel is offline, or update the title/thumbnail while live.
//!
//! This is a thin editor over the engine's `stream_control` write. The engine
//! stores the desired control; the platform adapters poll `stream_schedule` and
//! apply it, then report back via `stream_control_status`. The window renders
//! the engine's current control plus each adapter's last apply status (from the
//! same `stream_schedule` poll), so the operator can see what actually landed.
//!
//! The thumbnail field accepts a dragged file: the terminal delivers it as a
//! bracketed paste (see [`Window::editor_paste`]).

use chrono::{Local, NaiveDateTime, TimeZone};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::{PendingPrompt, Window};
use crate::colors::ColorConfig;
use crate::db::GlobalStats;
use crate::hotkeys::{Action, HotkeyConfig};

/// Which editable field has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Schedule,
    Thumbnail,
}

/// Parse a local-time schedule string ("YYYY-MM-DD HH:MM", `T` also accepted)
/// into epoch milliseconds. Empty input means "no schedule" (`0`); an
/// unparseable string returns `None`.
fn parse_local_datetime(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return Some(0);
    }
    let normalized = s.replace('T', " ");
    let naive = NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S"))
        .ok()?;
    // `earliest()` resolves a DST-ambiguous local time deterministically.
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.timestamp_millis())
}

/// Format epoch milliseconds as a local "YYYY-MM-DD HH:MM" string. `0` (no
/// schedule) yields an empty string.
fn format_local_datetime(ms: i64) -> String {
    if ms == 0 {
        return String::new();
    }
    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub struct StreamManagerWindow {
    focus: Field,
    title: String,
    schedule: String,
    thumbnail: String,
    /// Set once the operator edits a field, so an incoming poll does not clobber
    /// their in-progress input.
    dirty: bool,
    /// The engine revision the buffers were last loaded from.
    loaded_revision: Option<u64>,
    /// A validation error from the last submit attempt.
    error: Option<String>,
}

impl StreamManagerWindow {
    pub fn new() -> Self {
        Self {
            focus: Field::Title,
            title: String::new(),
            schedule: String::new(),
            thumbnail: String::new(),
            dirty: false,
            loaded_revision: None,
            error: None,
        }
    }

    /// Copy the engine's current control into the buffers, unless the operator
    /// is mid-edit (`dirty`) or nothing changed.
    fn sync_from_engine(&mut self, stats: &GlobalStats) {
        let s = &stats.stream_schedule;
        if !self.dirty && self.loaded_revision != Some(s.revision) {
            self.title = s.title.clone();
            self.schedule = format_local_datetime(s.scheduled_start_ms);
            self.thumbnail = s.thumbnail_path.clone();
            self.loaded_revision = Some(s.revision);
            self.error = None;
        }
    }

    fn field_mut(&mut self) -> &mut String {
        match self.focus {
            Field::Title => &mut self.title,
            Field::Schedule => &mut self.schedule,
            Field::Thumbnail => &mut self.thumbnail,
        }
    }

    fn next_field(&mut self) {
        self.focus = match self.focus {
            Field::Title => Field::Schedule,
            Field::Schedule => Field::Thumbnail,
            Field::Thumbnail => Field::Title,
        };
    }

    fn prev_field(&mut self) {
        self.focus = match self.focus {
            Field::Title => Field::Thumbnail,
            Field::Schedule => Field::Title,
            Field::Thumbnail => Field::Schedule,
        };
    }

    /// Build the `stream_control` write from the buffers. An unparseable
    /// schedule leaves the buffers alone and sets an error.
    fn submit(&mut self) -> Option<Action> {
        let scheduled_start_ms = match parse_local_datetime(&self.schedule) {
            Some(ms) => ms,
            None => {
                self.error = Some("start must be 'YYYY-MM-DD HH:MM' (local) or empty".to_string());
                return Some(Action::Noop);
            }
        };
        self.error = None;
        self.dirty = false;
        let payload = serde_json::json!({
            "title": self.title,
            "scheduled_start_ms": scheduled_start_ms,
            "thumbnail_path": self.thumbnail,
        });
        Some(Action::UserQuery("stream_control".to_string(), payload.to_string()))
    }

    fn hotkey_text(&self) -> String {
        "field:[Tab]  edit:[type]  submit:[Enter]  clear:[Del]  paste:[drag file]  quit:[esc]".to_string()
    }

    fn render_field(&self, area: Rect, buf: &mut Buffer, label: &str, value: &str, focused: bool) {
        let label_style = if focused {
            Style::default().fg(Color::Black).bg(Color::LightGreen).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };
        let value_style = if focused {
            Style::default().fg(Color::Black).bg(Color::LightGreen)
        } else {
            Style::default().fg(Color::White)
        };
        let cursor = if focused { "▏" } else { "" };
        // Truncate the value to the available width (a thumbnail path is long).
        let avail = (area.width as usize).saturating_sub(13);
        let shown: String = value.chars().take(avail).collect();
        let line = Line::from(vec![
            Span::styled(format!(" {:<10}", label), label_style),
            Span::styled(shown, value_style),
            Span::styled(cursor.to_string(), value_style),
        ]);
        line.render(area, buf);
    }
}

impl Window for StreamManagerWindow {
    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        is_active: bool,
        stats: &GlobalStats,
        colors: &ColorConfig,
        _hotkeys: &HotkeyConfig,
        _prompts: &[PendingPrompt],
    ) {
        self.sync_from_engine(stats);

        let border_color = if is_active {
            colors.active_border_color("stream_manager")
        } else {
            colors.border_color("inactive")
        };
        let block = Block::default()
            .title(" stream manager ")
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });
        let mut y = inner.y;

        // The engine's current control + revision.
        let s = &stats.stream_schedule;
        let title = if s.title.is_empty() { "(no title)".to_string() } else { s.title.clone() };
        let start = if s.scheduled_start_ms == 0 {
            "—".to_string()
        } else {
            format_local_datetime(s.scheduled_start_ms)
        };
        let header = Line::from(vec![
            Span::styled(" engine ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!("rev {}  ", s.revision), Style::default().fg(Color::DarkGray)),
            Span::styled(title, Style::default().fg(Color::White)),
            Span::styled(format!("   start {}", start), Style::default().fg(Color::DarkGray)),
        ]);
        header.render(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf);
        y += 2;

        self.render_field(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf, "title", &self.title, self.focus == Field::Title);
        y += 1;
        self.render_field(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf, "start", &self.schedule, self.focus == Field::Schedule);
        y += 1;
        self.render_field(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf, "thumbnail", &self.thumbnail, self.focus == Field::Thumbnail);
        y += 1;

        if let Some(err) = &self.error {
            let line = Line::from(Span::styled(format!("  {}", err), Style::default().fg(Color::LightRed)));
            line.render(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf);
            y += 1;
        }
        y += 1;

        let status_header = Line::from(Span::styled(
            " applied by adapters",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        status_header.render(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf);
        y += 1;
        if s.statuses.is_empty() {
            let line = Line::from(Span::styled(
                "   (no adapter has applied this revision yet)",
                Style::default().fg(Color::DarkGray),
            ));
            line.render(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf);
        }
        for st in &s.statuses {
            if y >= inner.y + inner.height {
                break;
            }
            let (tag, color) = if st.ok {
                ("ok", Color::Green)
            } else {
                ("FAIL", Color::LightRed)
            };
            let line = Line::from(vec![
                Span::styled(format!("   {:<16} ", st.platform), Style::default().fg(Color::White)),
                Span::styled(format!("{:<5}", tag), Style::default().fg(color)),
                Span::styled(st.message.clone(), Style::default().fg(Color::DarkGray)),
            ]);
            line.render(Rect { x: inner.x, y, width: inner.width, height: 1 }, buf);
            y += 1;
        }

        block.render(area, buf);

        let hotkey = crate::windows::hotkey_wrap::layout(
            &[(self.hotkey_text(), Style::default().fg(Color::DarkGray))],
            area,
            inner,
        );
        Paragraph::new(hotkey.lines).render(hotkey.area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent, _stats: &mut GlobalStats) -> Option<Action> {
        match key.code {
            KeyCode::Tab | KeyCode::Down => {
                self.next_field();
                Some(Action::Noop)
            }
            KeyCode::BackTab | KeyCode::Up => {
                self.prev_field();
                Some(Action::Noop)
            }
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.field_mut().pop();
                self.dirty = true;
                Some(Action::Noop)
            }
            KeyCode::Delete => {
                self.field_mut().clear();
                self.dirty = true;
                Some(Action::Noop)
            }
            KeyCode::Char(c)
                if key.kind != KeyEventKind::Release
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.field_mut().push(c);
                self.dirty = true;
                Some(Action::Noop)
            }
            KeyCode::Esc => Some(Action::Quit),
            _ => None,
        }
    }

    fn editor_paste(&mut self, text: &str) -> bool {
        let cleaned = text.trim_end_matches(['\r', '\n']);
        if cleaned.is_empty() {
            return false;
        }
        self.field_mut().push_str(cleaned);
        self.dirty = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::default_colors;
    use crate::hotkeys::default_hotkeys;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    #[test]
    fn empty_schedule_means_none_and_garbage_is_rejected() {
        assert_eq!(parse_local_datetime(""), Some(0));
        assert_eq!(parse_local_datetime("   "), Some(0));
        assert_eq!(parse_local_datetime("not a date"), None);
        assert_eq!(parse_local_datetime("2026-13-45 99:99"), None);
    }

    #[test]
    fn schedule_round_trips_through_local_format() {
        let ms = parse_local_datetime("2026-10-09 20:30").expect("valid datetime");
        assert!(ms > 0);
        assert_eq!(format_local_datetime(ms), "2026-10-09 20:30");
        // The `T` separator is accepted too.
        assert_eq!(parse_local_datetime("2026-10-09T20:30"), Some(ms));
        // `0` formats to empty (no schedule).
        assert_eq!(format_local_datetime(0), "");
    }

    #[test]
    fn typing_then_submit_sends_a_stream_control_write() {
        let mut w = StreamManagerWindow::new();
        let mut stats = GlobalStats::default();
        for c in "Coffee & Code".chars() {
            assert_eq!(w.handle_key(key(c), &mut stats), Some(Action::Noop));
        }
        // Move to the start field and type a datetime.
        assert_eq!(w.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut stats), Some(Action::Noop));
        for c in "2026-10-09 20:00".chars() {
            w.handle_key(key(c), &mut stats);
        }
        // Move to the thumbnail field and paste a dragged path.
        assert_eq!(w.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::empty()), &mut stats), Some(Action::Noop));
        assert_eq!(w.focus, Field::Thumbnail);
        assert!(w.editor_paste("/Users/x/thumb.png\n"));
        assert_eq!(w.thumbnail, "/Users/x/thumb.png");

        let action = w.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()), &mut stats);
        match action {
            Some(Action::UserQuery(qid, sql)) => {
                assert_eq!(qid, "stream_control");
                let v: serde_json::Value = serde_json::from_str(&sql).unwrap();
                assert_eq!(v["title"], "Coffee & Code");
                assert_eq!(v["thumbnail_path"], "/Users/x/thumb.png");
                assert_eq!(v["scheduled_start_ms"], parse_local_datetime("2026-10-09 20:00").unwrap());
            }
            other => panic!("expected stream_control, got {:?}", other),
        }
        assert!(!w.dirty, "submit clears dirty so the next poll can sync");
    }

    #[test]
    fn an_invalid_schedule_blocks_submit() {
        let mut w = StreamManagerWindow::new();
        let mut stats = GlobalStats::default();
        w.focus = Field::Schedule;
        for c in "whenever".chars() {
            w.handle_key(key(c), &mut stats);
        }
        let action = w.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()), &mut stats);
        assert_eq!(action, Some(Action::Noop), "invalid schedule must not submit");
        assert!(w.error.is_some());
    }

    #[test]
    fn a_poll_does_not_clobber_an_in_progress_edit() {
        let mut w = StreamManagerWindow::new();
        let mut stats = GlobalStats::default();
        stats.stream_schedule.title = "engine title".into();
        stats.stream_schedule.revision = 1;
        // First render syncs from the engine.
        let colors = default_colors();
        let hotkeys = default_hotkeys();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 20));
        w.render(Rect::new(0, 0, 80, 20), &mut buf, true, &stats, &colors, &hotkeys, &[]);
        assert_eq!(w.title, "engine title");

        // The operator edits; a later poll with a NEW revision must not clobber.
        w.handle_key(key('X'), &mut stats);
        assert_eq!(w.title, "engine titleX");
        stats.stream_schedule.revision = 2;
        stats.stream_schedule.title = "changed elsewhere".into();
        w.render(Rect::new(0, 0, 80, 20), &mut buf, true, &stats, &colors, &hotkeys, &[]);
        assert_eq!(w.title, "engine titleX", "in-progress edit preserved");
    }

    #[test]
    fn render_shows_the_engine_control_and_statuses() {
        let mut w = StreamManagerWindow::new();
        let mut stats = GlobalStats::default();
        stats.stream_schedule.title = "Launch day".into();
        stats.stream_schedule.revision = 3;
        stats.stream_schedule.statuses = vec![crate::db::StreamControlStatusView {
            platform: "youtube".into(),
            revision: 3,
            ok: true,
            message: "title set".into(),
        }];
        let colors = default_colors();
        let hotkeys = default_hotkeys();
        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 24));
        w.render(Rect::new(0, 0, 100, 24), &mut buf, true, &stats, &colors, &hotkeys, &[]);
        let mut all = String::new();
        for y in 0..24u16 {
            for x in 0..100u16 {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(all.contains("Launch day"), "engine title missing:\n{}", all);
        assert!(all.contains("youtube"), "adapter status missing:\n{}", all);
        assert!(all.contains("title set"), "status message missing:\n{}", all);
    }
}

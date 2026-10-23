//! Audit view: the engine's held-for-review messages (`pipeline_status =
//! 'audit'`), rendered as an in-TUI pane. The operator navigates the list and
//! approves (resubmit as a normal message) or rejects (mark failed) a row.
//!
//! The data comes from the engine's TUI-only `audit_list` query, polled only
//! while an audit pane is visible (see `db::extended_queries` /
//! `main::poll_set_for`). Approve/reject are one-shot `audit_approve` /
//! `audit_reject` writes followed by an immediate `audit_list` refresh, so the
//! list updates promptly instead of waiting for the next poll tick.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::Window;
use crate::colors::ColorConfig;
use crate::db::{AuditEntry, GlobalStats};
use crate::hotkeys::{Action, HotkeyConfig};

/// The audit pane. Holds only its own cursor/scroll; the rows live in
/// `GlobalStats` (polled by the engine client).
pub struct AuditWindow {
    /// Index into `stats.audit_entries`.
    selected: usize,
    scroll: usize,
}

impl AuditWindow {
    pub fn new() -> Self {
        Self { selected: 0, scroll: 0 }
    }

    fn clamp_selection(&mut self, len: usize) {
        if len == 0 {
            self.selected = 0;
            self.scroll = 0;
        } else {
            self.selected = self.selected.min(len - 1);
            self.scroll = self.scroll.min(len.saturating_sub(1));
        }
    }

    /// The uuid7 of the selected held message, if any.
    fn selected_uuid(&self, stats: &GlobalStats) -> Option<String> {
        stats.audit_entries.get(self.selected).map(|e| e.uuid7.clone())
    }

    fn platform_color(platform: &str) -> Color {
        match platform {
            "twitch" => Color::Magenta,
            "youtube" => Color::Red,
            "kick" => Color::Green,
            "discord" => Color::Blue,
            _ => Color::Cyan,
        }
    }

    fn render_rows(&mut self, area: Rect, buf: &mut Buffer, stats: &GlobalStats) {
        let list = &stats.audit_entries;
        if list.is_empty() {
            let text = if stats.audit_epoch == 0 {
                "  (loading held messages…)"
            } else {
                "  (nothing held for review)"
            };
            Line::from(Span::styled(text, Style::default().fg(Color::DarkGray)))
                .render(Rect { x: area.x, y: area.y, width: area.width, height: 1 }, buf);
            return;
        }
        if area.height == 0 {
            return;
        }

        let visible = area.height as usize;
        let max_scroll = list.len().saturating_sub(visible);
        let scroll = self.scroll.min(max_scroll);
        let scroll = if self.selected < scroll {
            self.selected
        } else if self.selected >= scroll + visible {
            self.selected.saturating_add(1).saturating_sub(visible)
        } else {
            scroll
        };
        self.scroll = scroll;

        for (i, entry) in list.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            render_entry_row(entry, idx, idx == self.selected, Rect { x: area.x, y, width: area.width, height: 1 }, buf);
        }
    }
}

/// Draw one held-message row: `# platform user age reason message`.
fn render_entry_row(entry: &AuditEntry, idx: usize, is_selected: bool, area: Rect, buf: &mut Buffer) {
    let style = row_style(is_selected);
    let fg = |c: Color| if is_selected { Color::Black } else { c };
    let line = Line::from(vec![
        Span::styled(format!(" {:>3} ", idx + 1), style.fg(fg(Color::DarkGray))),
        Span::styled(
            format!("{:<9}", truncate(&entry.platform, 9)),
            style.fg(fg(AuditWindow::platform_color(&entry.platform))),
        ),
        Span::styled(format!("{:<14}", truncate(&entry.user_uuid7, 14)), style.fg(fg(Color::Yellow))),
        Span::styled(format!("{:<6}", relative_age(entry.persisted_at)), style.fg(fg(Color::DarkGray))),
        Span::styled(format!("{:<16}", truncate(&entry.reason, 16)), style.fg(fg(Color::Magenta))),
        Span::styled(truncate(&entry.raw_message, 60), style.fg(fg(Color::White))),
    ]);
    line.render(area, buf);
}

fn row_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default().fg(Color::Black).bg(Color::Cyan).add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// A held message's age as a short string (`12s`, `5m`, `3h`). Empty when the
/// engine did not persist a timestamp.
fn relative_age(persisted_at: Option<i64>) -> String {
    let Some(ms) = persisted_at else { return String::new() };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let diff = ((now - ms) / 1000).max(0);
    if diff < 60 {
        format!("{}s", diff)
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else {
        format!("{}h", diff / 3600)
    }
}

impl Window for AuditWindow {
    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        is_active: bool,
        stats: &GlobalStats,
        colors: &ColorConfig,
        hotkeys: &HotkeyConfig,
        _prompts: &[crate::app::PendingPrompt],
    ) {
        let count = stats.audit_entries.len();
        self.clamp_selection(count);

        let border_color = if is_active {
            colors.active_border_color("audit")
        } else {
            colors.border_color("inactive")
        };
        let block = Block::default()
            .title(format!(" audit ({}) ", count))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

        // Column header, then the rows, then the hotkey bar pinned to the
        // bottom (which may claim extra rows on a narrow pane).
        let hotkey = hotkeys.format_window(
            "audit",
            &["nav", "approve", "reject", "audit-refresh"],
        );
        let bar = crate::windows::hotkey_wrap::layout(
            &[(
                format!("nav:[j|k]  {}", if hotkey.is_empty() { "approve:[a]  reject:[x]  refresh:[r]".to_string() } else { hotkey }),
                Style::default().fg(Color::DarkGray),
            )],
            area,
            inner,
        );

        let header = Line::from(Span::styled(
            format!(" {:>3} {:<9}{:<14}{:<6}{:<16}{}", "#", "platform", "user", "age", "reason", "message"),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));

        if inner.height > 0 {
            header.render(Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 }, buf);
            let rows_area = Rect {
                x: bar.content.x,
                y: inner.y + 1,
                width: bar.content.width,
                height: bar.content.height.saturating_sub(1),
            };
            self.render_rows(rows_area, buf, stats);
        }

        block.render(area, buf);
        Paragraph::new(bar.lines).render(bar.area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent, stats: &mut GlobalStats) -> Option<Action> {
        let len = stats.audit_entries.len();
        self.clamp_selection(len);

        match key.code {
            KeyCode::Esc => Some(Action::Quit),
            KeyCode::Char('j') | KeyCode::Down => {
                if len > 0 {
                    self.selected = (self.selected + 1).min(len - 1);
                }
                Some(Action::Noop)
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                Some(Action::Noop)
            }
            KeyCode::Char('g') => {
                self.selected = 0;
                Some(Action::Noop)
            }
            KeyCode::Char('G') => {
                self.selected = len.saturating_sub(1);
                Some(Action::Noop)
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(len.saturating_sub(1));
                Some(Action::Noop)
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                Some(Action::Noop)
            }
            // Approve/reject/refresh are normally resolved through the key map
            // (which lets the operator rebind them); handled here too so the
            // window still works if a keymap omits them.
            KeyCode::Char('a') => Some(Action::AuditApprove(self.selected_uuid(stats).unwrap_or_default())),
            KeyCode::Char('x') => Some(Action::AuditReject(self.selected_uuid(stats).unwrap_or_default())),
            KeyCode::Char('r') => Some(Action::AuditRefresh),
            _ => None,
        }
    }

    fn selected_module_name(&self, _stats: &GlobalStats) -> Option<String> {
        None
    }

    fn selection_is_module(&self, _stats: &GlobalStats) -> bool {
        // The selected row is a held MESSAGE, never a module: a module-scoped
        // action pressed here must be refused rather than fall back to the
        // first known module.
        false
    }

    fn selected_audit_uuid(&self, stats: &GlobalStats) -> Option<String> {
        self.selected_uuid(stats)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::colors::default_colors;
    use crate::hotkeys::default_hotkeys;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), crossterm::event::KeyModifiers::empty())
    }

    fn entry(uuid: &str, platform: &str, msg: &str) -> AuditEntry {
        AuditEntry {
            uuid7: uuid.to_string(),
            platform: platform.to_string(),
            raw_message: msg.to_string(),
            user_uuid7: "user-1".to_string(),
            reason: "flagged".to_string(),
            persisted_at: Some(0),
        }
    }

    fn stats_with(entries: Vec<AuditEntry>) -> GlobalStats {
        GlobalStats {
            audit_entries: entries,
            audit_epoch: 1,
            ..Default::default()
        }
    }

    #[test]
    fn navigation_moves_and_clamps() {
        let mut w = AuditWindow::new();
        let mut stats = stats_with(vec![entry("u1", "twitch", "a"), entry("u2", "youtube", "b"), entry("u3", "kick", "c")]);

        assert_eq!(w.handle_key(key('j'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 1);
        assert_eq!(w.handle_key(key('k'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 0);
        assert_eq!(w.handle_key(key('G'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 2);
        assert_eq!(w.handle_key(key('g'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 0);
        // Navigation can never run past the end.
        w.selected = 2;
        assert_eq!(w.handle_key(key('j'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 2);
        assert_eq!(w.handle_key(KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::empty()), &mut stats), Some(Action::Quit));
    }

    #[test]
    fn approve_and_reject_carry_the_selected_uuid() {
        let mut w = AuditWindow::new();
        let mut stats = stats_with(vec![entry("uuid-a", "twitch", "a"), entry("uuid-b", "youtube", "b")]);
        w.selected = 1;
        assert_eq!(w.handle_key(key('a'), &mut stats), Some(Action::AuditApprove("uuid-b".to_string())));
        assert_eq!(w.handle_key(key('x'), &mut stats), Some(Action::AuditReject("uuid-b".to_string())));
        assert_eq!(w.handle_key(key('r'), &mut stats), Some(Action::AuditRefresh));
        // The trait accessor mirrors the selected uuid.
        assert_eq!(w.selected_audit_uuid(&stats).as_deref(), Some("uuid-b"));
    }

    #[test]
    fn an_empty_list_approve_is_inert() {
        let mut w = AuditWindow::new();
        let mut stats = stats_with(Vec::new());
        assert_eq!(w.handle_key(key('a'), &mut stats), Some(Action::AuditApprove(String::new())));
        assert!(w.selected_audit_uuid(&stats).is_none());
        assert!(!w.selection_is_module(&stats));
    }

    #[test]
    fn render_draws_the_held_rows_and_title() {
        let mut w = AuditWindow::new();
        let stats = stats_with(vec![entry("uuid-1", "twitch", "suspicious message")]);
        let colors = default_colors();
        let hotkeys = default_hotkeys();
        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 12));
        w.render(Rect::new(0, 0, 80, 12), &mut buf, true, &stats, &colors, &hotkeys, &[]);

        let mut all = String::new();
        for y in 0..12u16 {
            for x in 0..80u16 {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(all.contains("audit (1)"), "title missing:\n{}", all);
        assert!(all.contains("suspicious message"), "row missing:\n{}", all);
        assert!(all.contains("twitch"), "platform missing:\n{}", all);
    }
}

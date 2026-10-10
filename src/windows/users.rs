use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget};

use crate::app::Window;
use crate::colors::ColorConfig;
use crate::db::{GlobalStats, LeaderboardEntry};
use crate::hotkeys::{Action, HotkeyConfig};

/// The detached "top users" window: a plain list of the current-stream
/// leaderboard — each row is a chatter and the points they earned this stream.
///
/// The order is the `stream-leaderboard` module's (points earned, highest
/// first); the TUI never re-sorts, so the window and the engine agree on the
/// ranking. The user-db rows the parent polls are used only to colour each name
/// by its rank tier.
pub struct UsersWindow {
    /// Index into the filtered list.
    selected: usize,
    /// Scroll offset for the list.
    scroll: usize,
    /// Substring filter typed after `/`.
    filter: String,
    /// True while typing the filter (captures every key).
    filtering: bool,
}

impl UsersWindow {
    pub fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            filter: String::new(),
            filtering: false,
        }
    }

    // ── state helpers ───────────────────────────────────────────────

    /// The current-stream leaderboard, filtered by the typed filter.
    fn filtered_entries<'a>(&self, stats: &'a GlobalStats) -> Vec<&'a LeaderboardEntry> {
        if self.filter.is_empty() {
            stats.leaderboard.iter().collect()
        } else {
            let f = self.filter.to_lowercase();
            stats
                .leaderboard
                .iter()
                .filter(|e| e.username.to_lowercase().contains(&f))
                .collect()
        }
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

    fn rank_color(&self, tier: &str) -> Color {
        match tier {
            "coal" => Color::DarkGray,
            "copper" => Color::Rgb(184, 115, 51),
            "bronze" => Color::Rgb(205, 127, 50),
            "silver" => Color::Gray,
            "gold" => Color::Yellow,
            "sapphire" => Color::Blue,
            "emerald" => Color::Green,
            "ruby" => Color::Red,
            "diamond" => Color::LightCyan,
            "opal" => Color::Cyan,
            // Roles + any custom streamer tier name.
            "owner" => Color::LightRed,
            "admin" => Color::Magenta,
            "mod" => Color::LightGreen,
            "sponsor" => Color::LightYellow,
            _ => Color::White,
        }
    }

    // ── key handling ────────────────────────────────────────────────

    fn filter_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Enter => {
                // Commit the filter: keep it, exit typing mode.
                self.filtering = false;
                self.selected = 0;
                Some(Action::Noop)
            }
            KeyCode::Esc => {
                // Clear the filter entirely.
                self.filtering = false;
                self.filter.clear();
                self.selected = 0;
                Some(Action::Noop)
            }
            KeyCode::Char(c)
                if key.kind != KeyEventKind::Release
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.filter.push(c);
                self.selected = 0;
                Some(Action::Noop)
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.selected = 0;
                Some(Action::Noop)
            }
            _ => Some(Action::Noop),
        }
    }

    // ── rendering ───────────────────────────────────────────────────

    fn render_rows(&mut self, area: Rect, buf: &mut Buffer, stats: &GlobalStats) {
        let list = self.filtered_entries(stats);
        if list.is_empty() {
            let line = Line::from(Span::styled(
                if stats.leaderboard.is_empty() {
                    "  (no chatters yet this stream)"
                } else {
                    "  (no chatters match filter)"
                },
                Style::default().fg(Color::DarkGray),
            ));
            line.render(Rect { x: area.x, y: area.y, width: area.width, height: 1 }, buf);
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
            let is_selected = idx == self.selected;
            // Colour the name by the user's tier when the user-db row is known.
            let tier = stats
                .users
                .iter()
                .find(|u| u.uuid7 == entry.uuid)
                .map(|u| u.rank_tier())
                .unwrap_or_default();
            let name_color = if is_selected {
                Color::Black
            } else if tier.is_empty() {
                Color::White
            } else {
                self.rank_color(&tier)
            };
            let earned_color = if is_selected {
                Color::Black
            } else if entry.earned > 0 {
                Color::LightGreen
            } else if entry.earned < 0 {
                Color::LightRed
            } else {
                Color::DarkGray
            };
            let line = Line::from(vec![
                Span::styled(
                    format!(" {:>3} ", idx + 1),
                    row_style(is_selected).fg(if is_selected { Color::Black } else { Color::DarkGray }),
                ),
                Span::styled(
                    format!("{:<22}", truncate(&entry.username, 22)),
                    row_style(is_selected).fg(name_color),
                ),
                Span::styled(
                    format!("{:>8}", format!("{:+}", entry.earned)),
                    row_style(is_selected).fg(earned_color),
                ),
            ]);
            line.render(Rect { x: area.x, y, width: area.width, height: 1 }, buf);
        }
    }
}

/// Row style for the list: a selected row is inverted (cyan background).
fn row_style(is_selected: bool) -> Style {
    if is_selected {
        Style::default().fg(Color::Black).bg(Color::Cyan)
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

impl Window for UsersWindow {
    fn render(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        is_active: bool,
        stats: &GlobalStats,
        colors: &ColorConfig,
        _hotkeys: &HotkeyConfig,
        _prompts: &[crate::app::PendingPrompt],
    ) {
        let list = self.filtered_entries(stats);
        self.clamp_selection(list.len());

        let border_color = if is_active {
            colors.active_border_color("users")
        } else {
            colors.border_color("inactive")
        };
        let block = Block::default()
            .title(format!(" top users ({}) ", list.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));

        let inner = area.inner(Margin { horizontal: 1, vertical: 1 });

        // The column header (one row), then the rows, then the filter/error
        // lines pinned to the bottom.
        let header = Line::from(Span::styled(
            format!(" {:>3} {:<22}{:>8}", "#", "user", "earned"),
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ));
        header.render(Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 }, buf);

        let mut reserved = 0u16;
        if self.filtering {
            reserved += 1;
        }
        if stats.user_last_error.is_some() {
            reserved += 1;
        }
        let rows_area = Rect {
            x: inner.x,
            y: inner.y + 1,
            width: inner.width,
            height: inner.height.saturating_sub(1 + reserved),
        };
        self.render_rows(rows_area, buf, stats);

        // Filter line / error line (last rows inside the border).
        if self.filtering {
            let filter_line = Line::from(vec![
                Span::styled(" filter: ", Style::default().fg(Color::Cyan)),
                Span::styled(&self.filter, Style::default().fg(Color::White)),
            ]);
            filter_line.render(
                Rect { x: inner.x, y: inner.y + inner.height.saturating_sub(2), width: inner.width, height: 1 },
                buf,
            );
        }
        if let Some(err) = &stats.user_last_error {
            let err_line = Line::from(Span::styled(
                format!("  userdb: {}", truncate(err, inner.width as usize)),
                Style::default().fg(Color::LightRed),
            ));
            err_line.render(
                Rect { x: inner.x, y: inner.y + inner.height.saturating_sub(1), width: inner.width, height: 1 },
                buf,
            );
        }

        block.render(area, buf);

        // Hotkey bar: wrapped to this window's width.
        let hotkey = crate::windows::hotkey_wrap::layout(
            &[(
                "nav:[j|k|g|G]  filter:[/]  quit:[q|esc]".to_string(),
                Style::default().fg(Color::DarkGray),
            )],
            area,
            inner,
        );
        Paragraph::new(hotkey.lines).render(hotkey.area, buf);
    }

    fn handle_key(&mut self, key: KeyEvent, stats: &mut GlobalStats) -> Option<Action> {
        // An active filter captures every key.
        if self.filtering {
            return self.filter_key(key);
        }

        let list = self.filtered_entries(stats);
        self.clamp_selection(list.len());

        match key.code {
            KeyCode::Esc => Some(Action::Quit),
            KeyCode::Char('j') | KeyCode::Down => {
                if !list.is_empty() {
                    self.selected = (self.selected + 1).min(list.len() - 1);
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
                self.selected = list.len().saturating_sub(1);
                Some(Action::Noop)
            }
            KeyCode::PageDown => {
                self.selected = (self.selected + 10).min(list.len().saturating_sub(1));
                Some(Action::Noop)
            }
            KeyCode::PageUp => {
                self.selected = self.selected.saturating_sub(10);
                Some(Action::Noop)
            }
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter.clear();
                Some(Action::Noop)
            }
            _ => None,
        }
    }

    fn selected_module_name(&self, _stats: &GlobalStats) -> Option<String> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Window;
    use crate::colors::default_colors;
    use crate::db::{LeaderboardEntry, UserSummary};
    use crate::hotkeys::default_hotkeys;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    fn user(uuid: &str, name: &str, score: i32, rank: f32) -> UserSummary {
        UserSummary {
            uuid7: uuid.to_string(),
            username: name.to_string(),
            is_sponsor: false,
            is_moderator: false,
            is_admin: false,
            is_owner: false,
            score,
            commendations: 0,
            reprimands: 0,
            channels: Vec::new(),
            flags: "{}".to_string(),
            total_score: score,
            messages_sent: 0,
            rank,
        }
    }

    fn entry(uuid: &str, name: &str, earned: i32, score: i32) -> LeaderboardEntry {
        LeaderboardEntry {
            uuid: uuid.to_string(),
            username: name.to_string(),
            earned,
            score,
        }
    }

    fn stats_with_users() -> GlobalStats {
        GlobalStats {
            users: vec![
                user("00000000-0000-7000-0000-000000000001", "alice", 60, 0.93),
                user("00000000-0000-7000-0000-000000000002", "bob", 25, 0.45),
                user("00000000-0000-7000-0000-000000000003", "carol", -8, 0.12),
                user("00000000-0000-7000-0000-000000000004", "dave", 3, 0.55),
            ],
            // The list renders the current-stream leaderboard (order = earned
            // desc, as the module reports it).
            leaderboard: vec![
                entry("00000000-0000-7000-0000-000000000001", "alice", 45, 60),
                entry("00000000-0000-7000-0000-000000000002", "bob", 20, 25),
                entry("00000000-0000-7000-0000-000000000003", "carol", 8, -8),
                entry("00000000-0000-7000-0000-000000000004", "dave", 3, 3),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn rank_tier_boundaries() {
        // The tier comes from the 0-1 rank via the root rank_chart.json
        // (mineral ladder, one tier per 0.1).
        assert_eq!(crate::rank_chart::tier_name(0.0), "coal");
        assert_eq!(crate::rank_chart::tier_name(0.09), "coal");
        assert_eq!(crate::rank_chart::tier_name(0.1), "copper");
        assert_eq!(crate::rank_chart::tier_name(0.2), "bronze");
        assert_eq!(crate::rank_chart::tier_name(0.45), "gold");
        assert_eq!(crate::rank_chart::tier_name(0.5), "sapphire");
        assert_eq!(crate::rank_chart::tier_name(0.9), "opal");
        assert_eq!(crate::rank_chart::tier_name(1.0), "opal");
    }

    #[test]
    fn navigation_moves_selection() {
        let mut w = UsersWindow::new();
        let mut stats = stats_with_users();

        assert_eq!(w.handle_key(key('j'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 1);

        assert_eq!(w.handle_key(key('k'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 0);

        assert_eq!(w.handle_key(key('G'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 3);

        assert_eq!(w.handle_key(key('g'), &mut stats), Some(Action::Noop));
        assert_eq!(w.selected, 0);

        // Esc quits.
        assert_eq!(
            w.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut stats),
            Some(Action::Quit)
        );
    }

    #[test]
    fn filter_restricts_list() {
        let mut w = UsersWindow::new();
        let mut stats = stats_with_users();

        // Start a filter, type "bo".
        assert_eq!(w.handle_key(key('/'), &mut stats), Some(Action::Noop));
        assert!(w.filtering);
        assert_eq!(w.handle_key(key('b'), &mut stats), Some(Action::Noop));
        assert_eq!(w.handle_key(key('o'), &mut stats), Some(Action::Noop));

        let filtered = w.filtered_entries(&stats);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].username, "bob");

        // Enter commits the filter.
        assert_eq!(w.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()), &mut stats), Some(Action::Noop));
        assert!(!w.filtering);
        assert_eq!(w.filtered_entries(&stats)[0].username, "bob");

        // Esc clears the filter.
        assert_eq!(w.handle_key(key('/'), &mut stats), Some(Action::Noop));
        assert_eq!(w.handle_key(key('q'), &mut stats), Some(Action::Noop));
        assert_eq!(w.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()), &mut stats), Some(Action::Noop));
        assert!(w.filter.is_empty());
        assert!(!w.filtering);
        assert_eq!(w.filtered_entries(&stats).len(), 4);
    }

    #[test]
    fn render_draws_list() {
        let mut w = UsersWindow::new();
        let stats = stats_with_users();
        let colors = default_colors();
        let hotkeys = default_hotkeys();

        let mut buf = Buffer::empty(Rect::new(0, 0, 60, 20));
        w.render(Rect::new(0, 0, 60, 20), &mut buf, true, &stats, &colors, &hotkeys, &[]);

        let mut all = String::new();
        for y in 0..20u16 {
            for x in 0..60u16 {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(all.contains("alice"), "list missing alice:\n{}", all);
        assert!(all.contains("top users"), "title missing:\n{}", all);
        assert!(all.contains("earned"), "header missing:\n{}", all);
        assert!(all.contains("+45"), "earned column missing:\n{}", all);
        // The detail pane / mod tools are gone.
        assert!(!all.contains("score"), "detail pane should be gone:\n{}", all);
    }
}

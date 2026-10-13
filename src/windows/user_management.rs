use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::{Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

use crate::app::Window;
use crate::colors::ColorConfig;
use crate::db::{GlobalStats, UserSummary};
use crate::hotkeys::{Action, HotkeyConfig};

/// The detached "user management" window: a fuzzy search bar at the top, a
/// scrollable list of every user in the middle, and hotkey hints at the bottom.
/// Selecting a user (Enter) opens a modal action menu for reprimands,
/// commendations and manual score adjustments.
///
/// The list source is the polled `userdb_list_users` result (`stats.users`),
/// which the engine already sorts by score DESC.
pub struct UserManagementWindow {
    /// Index into the filtered list.
    selected: usize,
    /// Scroll offset for the list.
    scroll: usize,
    /// Fuzzy search query typed after `/`.
    filter: String,
    /// True while typing the search (captures every key).
    filtering: bool,
    /// The open modal action menu / score editor, if any.
    dialog: Option<UserDialog>,
    /// The last action's result, shown above the hotkey bar.
    status: Option<String>,
}

/// The user-management modal. The `Menu` lists the per-user actions; the
/// `AdjustScore` arm is the small signed-integer editor opened by `s`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum UserDialog {
    Menu {
        uuid7: String,
        username: String,
    },
    AdjustScore {
        uuid7: String,
        username: String,
        input: String,
    },
}

/// Which rating a reprimand/commendation action applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RateKind {
    Reprimand,
    Commend,
}

impl UserManagementWindow {
    pub fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
            filter: String::new(),
            filtering: false,
            dialog: None,
            status: None,
        }
    }

    // ── state helpers ───────────────────────────────────────────────

    /// The users matching the current fuzzy query, best score first. An empty
    /// query keeps the engine's order (score DESC).
    fn filtered_users<'a>(&self, users: &'a [UserSummary]) -> Vec<&'a UserSummary> {
        if self.filter.trim().is_empty() {
            return users.iter().collect();
        }
        let mut scored: Vec<(usize, &UserSummary)> = users
            .iter()
            .filter_map(|u| {
                let score = fuzzy_match_user(&self.filter, u);
                (score > 0).then_some((score, u))
            })
            .collect();
        // Stable sort: equal scores keep the engine's score-DESC order.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.into_iter().map(|(_, u)| u).collect()
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

    fn rank_color(tier: &str) -> Color {
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
            "owner" => Color::LightRed,
            "admin" => Color::Magenta,
            "mod" => Color::LightGreen,
            "sponsor" => Color::LightYellow,
            _ => Color::White,
        }
    }

    /// Open the action menu for the user at the cursor of the filtered list.
    fn open_menu(&mut self, stats: &GlobalStats) {
        let list = self.filtered_users(&stats.users);
        let Some(user) = list.get(self.selected) else {
            return;
        };
        self.status = None;
        self.dialog = Some(UserDialog::Menu {
            uuid7: user.uuid7.clone(),
            username: user.username.clone(),
        });
    }

    // ── key handling ────────────────────────────────────────────────

    fn filter_key(&mut self, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Enter => {
                self.filtering = false;
                self.selected = 0;
                Some(Action::Noop)
            }
            KeyCode::Esc => {
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

    /// Resolve a key while the modal is open. Returns the action to dispatch
    /// (a `UserQuery` when a rating is applied), or `Noop` when the key only
    /// changed the modal's own state.
    fn dialog_key_impl(&mut self, key: KeyEvent) -> Option<Action> {
        let dialog = self.dialog.take()?;
        match dialog {
            UserDialog::Menu { uuid7, username } => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.status = None;
                    Some(Action::Noop)
                }
                KeyCode::Char('r') => {
                    self.status = Some(format!("reprimand sent for {}", username));
                    Some(rate_query(&uuid7, RateKind::Reprimand, ""))
                }
                KeyCode::Char('c') => {
                    self.status = Some(format!("commendation sent for {}", username));
                    Some(rate_query(&uuid7, RateKind::Commend, ""))
                }
                KeyCode::Char('s') => {
                    self.dialog = Some(UserDialog::AdjustScore {
                        uuid7,
                        username,
                        input: String::new(),
                    });
                    Some(Action::Noop)
                }
                KeyCode::Char('l') => {
                    // The merge ships later; the option stays visible but inert.
                    self.status = Some("link account — coming soon".to_string());
                    self.dialog = Some(UserDialog::Menu { uuid7, username });
                    Some(Action::Noop)
                }
                _ => {
                    self.dialog = Some(UserDialog::Menu { uuid7, username });
                    Some(Action::Noop)
                }
            },
            UserDialog::AdjustScore {
                uuid7,
                username,
                mut input,
            } => match key.code {
                KeyCode::Esc => {
                    // Back to the action menu rather than all the way out.
                    self.dialog = Some(UserDialog::Menu { uuid7, username });
                    Some(Action::Noop)
                }
                KeyCode::Enter => match parse_delta(&input) {
                    Some(delta) => {
                        self.status = Some(format!("score {:+} applied to {}", delta, username));
                        Some(adjust_score_query(&uuid7, delta))
                    }
                    None => {
                        self.status = Some("score must be a signed integer".to_string());
                        self.dialog = Some(UserDialog::AdjustScore {
                            uuid7,
                            username,
                            input,
                        });
                        Some(Action::Noop)
                    }
                },
                KeyCode::Backspace => {
                    input.pop();
                    self.dialog = Some(UserDialog::AdjustScore {
                        uuid7,
                        username,
                        input,
                    });
                    Some(Action::Noop)
                }
                KeyCode::Char(c)
                    if (c.is_ascii_digit() || c == '-') && input.chars().count() < 12 =>
                {
                    input.push(c);
                    self.dialog = Some(UserDialog::AdjustScore {
                        uuid7,
                        username,
                        input,
                    });
                    Some(Action::Noop)
                }
                _ => {
                    self.dialog = Some(UserDialog::AdjustScore {
                        uuid7,
                        username,
                        input,
                    });
                    Some(Action::Noop)
                }
            },
        }
    }

    // ── rendering ───────────────────────────────────────────────────

    fn render_rows(&mut self, area: Rect, buf: &mut Buffer, list: &[&UserSummary]) {
        if area.height == 0 {
            return;
        }
        if list.is_empty() {
            let line = Line::from(Span::styled(
                "  (no users match search)",
                Style::default().fg(Color::DarkGray),
            ));
            line.render(
                Rect {
                    x: area.x,
                    y: area.y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
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

        for (i, user) in list.iter().skip(scroll).take(visible).enumerate() {
            let idx = scroll + i;
            let y = area.y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let is_selected = idx == self.selected;
            let tier = user.rank_tier();
            let name_color = if is_selected {
                Color::Black
            } else if tier.is_empty() {
                Color::White
            } else {
                Self::rank_color(&tier)
            };
            let score_color = if is_selected {
                Color::Black
            } else if user.score > 0 {
                Color::LightGreen
            } else if user.score < 0 {
                Color::LightRed
            } else {
                Color::DarkGray
            };
            let line = Line::from(vec![
                Span::styled(
                    format!(" {:>3} ", idx + 1),
                    row_style(is_selected).fg(if is_selected {
                        Color::Black
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(
                    format!("{:<28}", truncate(&display_name(user), 28)),
                    row_style(is_selected).fg(name_color),
                ),
                Span::styled(
                    format!("{:>7}", user.score),
                    row_style(is_selected).fg(score_color),
                ),
                Span::styled(
                    format!("  {:<10}", truncate(&tier, 10)),
                    row_style(is_selected).fg(if is_selected {
                        Color::Black
                    } else {
                        Color::DarkGray
                    }),
                ),
            ]);
            line.render(
                Rect {
                    x: area.x,
                    y,
                    width: area.width,
                    height: 1,
                },
                buf,
            );
        }
    }
}

/// The engine virtual query that applies a reprimand or commendation from the
/// TUI. The `kind` field (`"commend"` / `"reprimand"`) selects which; the engine
/// records the operator as the giver (see `cockatiel_engine-rs/src/queries.rs`).
pub(crate) const RATE_QUERY_ID: &str = "userdb_rate_user";

/// Build the one-shot rating query for `uuid7`.
pub(crate) fn rate_query(uuid7: &str, kind: RateKind, reason: &str) -> Action {
    let kind = match kind {
        RateKind::Reprimand => "reprimand",
        RateKind::Commend => "commend",
    };
    let payload = serde_json::json!({ "uuid7": uuid7, "kind": kind, "reason": reason });
    Action::UserQuery(RATE_QUERY_ID.to_string(), payload.to_string())
}

/// Parse a signed integer typed into the score editor. `None` for empty or
/// malformed input (and for anything outside the 32-bit cap).
pub(crate) fn parse_delta(input: &str) -> Option<i32> {
    let t = input.trim();
    if t.is_empty() {
        return None;
    }
    t.parse::<i32>().ok()
}

/// Build the raw score-adjustment query. No multiplier is applied.
pub(crate) fn adjust_score_query(uuid7: &str, delta: i32) -> Action {
    let payload = serde_json::json!({
        "uuid7": uuid7,
        "delta": delta,
        "reason": "manual",
        "multiplier": false,
    });
    Action::UserQuery("userdb_adjust_score".to_string(), payload.to_string())
}

/// The fuzzy match score for a user: the best score across the cockatiel
/// username, the display identity and every channel identity.
pub(crate) fn fuzzy_match_user(query: &str, user: &UserSummary) -> usize {
    let mut best = crate::fuzzy::fuzzy_subsequence(query, &user.username);
    best = best.max(crate::fuzzy::fuzzy_subsequence(query, &display_name(user)));
    for channel in &user.channels {
        best = best.max(crate::fuzzy::fuzzy_subsequence(query, channel));
    }
    best
}

/// The user's display identity: `cockatiel_username:platform:username` when the
/// user has a channel, else just the cockatiel username.
pub(crate) fn display_name(user: &UserSummary) -> String {
    match user.channels.first() {
        Some(channel) => {
            let (platform, handle) = split_channel(channel);
            let handle = if handle.is_empty() {
                user.username.clone()
            } else {
                handle
            };
            format!("{}:{}:{}", user.username, platform, handle)
        }
        None => user.username.clone(),
    }
}

/// Split a stored channel string (`"platform:channel_id (handle)"`) into its
/// platform and handle.
fn split_channel(channel: &str) -> (String, String) {
    let platform = channel.split(':').next().unwrap_or("").to_string();
    let handle = channel
        .rsplit_once('(')
        .and_then(|(_, rest)| rest.strip_suffix(')'))
        .unwrap_or("")
        .to_string();
    (platform, handle)
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

/// Draw the modal over `area`. A no-op when no dialog is open.
fn render_dialog(dialog: &Option<UserDialog>, area: Rect, buf: &mut Buffer, colors: &ColorConfig) {
    let Some(dialog) = dialog else { return };
    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let (title, lines): (&str, Vec<(String, Style)>) = match dialog {
        UserDialog::Menu { username, .. } => (
            " actions ",
            vec![
                (format!(" User: {}", username), bold),
                (String::new(), Style::default()),
                (
                    " [r] give reprimand".to_string(),
                    Style::default().fg(Color::LightRed),
                ),
                (
                    " [c] give commendation".to_string(),
                    Style::default().fg(Color::LightGreen),
                ),
                (
                    " [s] adjust score (+/-)".to_string(),
                    Style::default().fg(Color::Cyan),
                ),
                (
                    " [l] link account — coming soon (disabled)".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
                (String::new(), Style::default()),
                (
                    " esc to cancel".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ],
        ),
        UserDialog::AdjustScore {
            username, input, ..
        } => (
            " adjust score ",
            vec![
                (format!(" Signed score delta for {}:", username), bold),
                (String::new(), Style::default()),
                (format!(" > {}", input), Style::default().fg(Color::White)),
                (String::new(), Style::default()),
                (
                    " Enter to apply   esc to cancel".to_string(),
                    Style::default().fg(Color::DarkGray),
                ),
            ],
        ),
    };

    let w = 56u16.min(area.width.saturating_sub(2)).max(24);
    let h = (lines.len() as u16 + 2)
        .min(area.height.saturating_sub(2))
        .max(3);
    let x = area.x + area.width.saturating_sub(w) / 2;
    let y = area.y + area.height.saturating_sub(h) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    // Clear the region first so the list does not show through the dialog.
    Clear.render(rect, buf);

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.active_border_color("user_management")));
    let inner = rect.inner(Margin {
        horizontal: 1,
        vertical: 1,
    });

    for (i, (text, style)) in lines.iter().enumerate() {
        if (i as u16) >= inner.height {
            break;
        }
        Line::from(Span::styled(text.clone(), *style)).render(
            Rect {
                x: inner.x,
                y: inner.y + i as u16,
                width: inner.width,
                height: 1,
            },
            buf,
        );
    }
    block.render(rect, buf);
}

impl Window for UserManagementWindow {
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
        let list = self.filtered_users(&stats.users);
        self.clamp_selection(list.len());

        let border_color = if is_active {
            colors.active_border_color("user_management")
        } else {
            colors.border_color("inactive")
        };
        let block = Block::default()
            .title(format!(" user management ({}) ", list.len()))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color));
        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 1,
        });

        let hints = if self.dialog.is_some() {
            "menu:[r|c|s|l]  cancel:[esc]".to_string()
        } else {
            "nav:[j|k|g|G]  search:[/]  select:[enter]  quit:[q|esc]".to_string()
        };
        let bar = crate::windows::hotkey_wrap::layout(
            &[(hints, Style::default().fg(Color::DarkGray))],
            area,
            inner,
        );
        let content = bar.content;

        // Band 1: the fuzzy search bar.
        if content.height >= 1 {
            let (label, value, value_style) = if self.filtering {
                (
                    " search: ",
                    self.filter.as_str(),
                    Style::default().fg(Color::White),
                )
            } else if self.filter.is_empty() {
                (
                    " search: ",
                    "(/ to search)",
                    Style::default().fg(Color::DarkGray),
                )
            } else {
                (
                    " search: ",
                    self.filter.as_str(),
                    Style::default().fg(Color::Yellow),
                )
            };
            Line::from(vec![
                Span::styled(
                    label,
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(value.to_string(), value_style),
            ])
            .render(
                Rect {
                    x: content.x,
                    y: content.y,
                    width: content.width,
                    height: 1,
                },
                buf,
            );
        }

        // Band 2: the column header + scrollable list.
        if content.height >= 2 {
            Line::from(Span::styled(
                format!(" {:>3} {:<28}{:>7}  {:<10}", "#", "user", "score", "rank"),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ))
            .render(
                Rect {
                    x: content.x,
                    y: content.y + 1,
                    width: content.width,
                    height: 1,
                },
                buf,
            );
        }

        let status_rows = u16::from(self.status.is_some());
        let list_area = Rect {
            x: content.x,
            y: content.y + 2,
            width: content.width,
            height: content.height.saturating_sub(2 + status_rows),
        };
        self.render_rows(list_area, buf, &list);

        // Band 3: the status line, pinned above the hotkey bar.
        if let Some(status) = &self.status {
            if content.height >= 1 {
                let style = if status.contains("coming soon") || status.contains("must be") {
                    Style::default().fg(Color::Yellow)
                } else {
                    Style::default().fg(Color::LightGreen)
                };
                Line::from(Span::styled(
                    format!("  {}", truncate(status, content.width as usize)),
                    style,
                ))
                .render(
                    Rect {
                        x: content.x,
                        y: content.y + content.height.saturating_sub(1),
                        width: content.width,
                        height: 1,
                    },
                    buf,
                );
            }
        }

        block.render(area, buf);
        Paragraph::new(bar.lines).render(bar.area, buf);

        // The modal draws last so it sits on top of the list.
        render_dialog(&self.dialog, area, buf, colors);
    }

    fn handle_key(&mut self, key: KeyEvent, stats: &mut GlobalStats) -> Option<Action> {
        if self.filtering {
            return self.filter_key(key);
        }

        let list = self.filtered_users(&stats.users);
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
                self.selected = 0;
                Some(Action::Noop)
            }
            KeyCode::Enter => {
                self.open_menu(stats);
                Some(Action::Noop)
            }
            _ => None,
        }
    }

    fn in_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    fn dialog_key(&mut self, key: KeyEvent) -> Option<Action> {
        self.dialog_key_impl(key)
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
    use crate::db::UserSummary;
    use crate::hotkeys::default_hotkeys;

    fn key(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::empty())
    }

    fn user(uuid: &str, name: &str, score: i32, rank: f32, channels: &[&str]) -> UserSummary {
        UserSummary {
            uuid7: uuid.to_string(),
            username: name.to_string(),
            score,
            rank,
            channels: channels.iter().map(|c| c.to_string()).collect(),
            ..Default::default()
        }
    }

    fn stats_with_users() -> GlobalStats {
        GlobalStats {
            users: vec![
                user("u1", "alice", 60, 0.93, &["twitch:1 (alice_tv)"]),
                user("u2", "bob", 25, 0.45, &[]),
                user("u3", "carol", -8, 0.12, &["youtube:2 (carol_yt)"]),
                user("u4", "dave", 3, 0.55, &[]),
            ],
            ..Default::default()
        }
    }

    #[test]
    fn fuzzy_filter_matches_username_and_channels() {
        let mut w = UserManagementWindow::new();
        let stats = stats_with_users();

        w.filter = "ali".to_string();
        let list = w.filtered_users(&stats.users);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].username, "alice");

        // A channel handle is searchable even when the cockatiel username is not.
        w.filter = "carol_yt".to_string();
        let list = w.filtered_users(&stats.users);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].username, "carol");

        // Fuzzy subsequence, not substring: "dve" still finds dave.
        w.filter = "dve".to_string();
        assert_eq!(w.filtered_users(&stats.users)[0].username, "dave");

        // No match drops the row.
        w.filter = "zzz".to_string();
        assert!(w.filtered_users(&stats.users).is_empty());

        // Empty filter keeps the engine order.
        w.filter.clear();
        let all = w.filtered_users(&stats.users);
        assert_eq!(
            all.iter().map(|u| u.username.as_str()).collect::<Vec<_>>(),
            vec!["alice", "bob", "carol", "dave"]
        );
    }

    #[test]
    fn display_name_qualifies_with_channel() {
        let with_channel = user("u1", "alice", 1, 0.5, &["twitch:1 (alice_tv)"]);
        assert_eq!(display_name(&with_channel), "alice:twitch:alice_tv");
        let without = user("u2", "bob", 1, 0.5, &[]);
        assert_eq!(display_name(&without), "bob");
    }

    #[test]
    fn enter_opens_the_action_menu_for_the_selected_user() {
        let mut w = UserManagementWindow::new();
        let mut stats = stats_with_users();

        w.handle_key(key('j'), &mut stats);
        assert_eq!(
            w.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
                &mut stats
            ),
            Some(Action::Noop)
        );
        assert!(w.in_dialog());
        assert_eq!(
            w.dialog,
            Some(UserDialog::Menu {
                uuid7: "u2".to_string(),
                username: "bob".to_string()
            })
        );
    }

    #[test]
    fn menu_reprimand_and_commendation_emit_rating_queries() {
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::Menu {
            uuid7: "u1".to_string(),
            username: "alice".to_string(),
        });

        let action = w.dialog_key(key('r')).unwrap();
        match action {
            Action::UserQuery(id, payload) => {
                assert_eq!(id, "userdb_rate_user");
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["uuid7"], "u1");
                assert_eq!(v["kind"], "reprimand");
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
        assert!(!w.in_dialog(), "the menu closes after a rating");

        w.dialog = Some(UserDialog::Menu {
            uuid7: "u1".to_string(),
            username: "alice".to_string(),
        });
        let action = w.dialog_key(key('c')).unwrap();
        match action {
            Action::UserQuery(id, payload) => {
                assert_eq!(id, "userdb_rate_user");
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["kind"], "commend");
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
    }

    #[test]
    fn adjust_score_parses_a_signed_integer_and_emits_the_query() {
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::Menu {
            uuid7: "u1".to_string(),
            username: "alice".to_string(),
        });

        // `s` opens the editor without emitting anything.
        assert_eq!(w.dialog_key(key('s')), Some(Action::Noop));
        assert!(matches!(w.dialog, Some(UserDialog::AdjustScore { .. })));

        assert_eq!(w.dialog_key(key('-')), Some(Action::Noop));
        assert_eq!(w.dialog_key(key('5')), Some(Action::Noop));
        let action = w
            .dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        match action {
            Action::UserQuery(id, payload) => {
                assert_eq!(id, "userdb_adjust_score");
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["uuid7"], "u1");
                assert_eq!(v["delta"], -5);
                assert_eq!(v["reason"], "manual");
                assert_eq!(v["multiplier"], false);
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
        assert!(!w.in_dialog());
    }

    #[test]
    fn adjust_score_rejects_malformed_input_and_keeps_the_editor_open() {
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::AdjustScore {
            uuid7: "u1".to_string(),
            username: "alice".to_string(),
            input: "-".to_string(),
        });
        assert_eq!(
            w.dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            Some(Action::Noop)
        );
        assert!(w.in_dialog(), "a malformed delta must not close the editor");
        assert!(w.status.as_deref().unwrap().contains("signed integer"));
        assert_eq!(parse_delta(""), None);
        assert_eq!(parse_delta("abc"), None);
        assert_eq!(parse_delta("+7"), Some(7));
    }

    #[test]
    fn link_account_is_disabled_and_keeps_the_menu_open() {
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::Menu {
            uuid7: "u1".to_string(),
            username: "alice".to_string(),
        });
        assert_eq!(w.dialog_key(key('l')), Some(Action::Noop));
        assert!(w.in_dialog(), "link account is a coming-soon no-op");
        assert!(w.status.as_deref().unwrap().contains("coming soon"));
    }

    #[test]
    fn render_draws_search_list_and_hints() {
        let mut w = UserManagementWindow::new();
        let stats = stats_with_users();
        let colors = default_colors();
        let hotkeys = default_hotkeys();

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        w.render(
            Rect::new(0, 0, 80, 24),
            &mut buf,
            true,
            &stats,
            &colors,
            &hotkeys,
            &[],
        );

        let mut all = String::new();
        for y in 0..24u16 {
            for x in 0..80u16 {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(all.contains("user management"), "title missing:\n{}", all);
        assert!(all.contains("search"), "search bar missing:\n{}", all);
        assert!(all.contains("alice"), "list missing alice:\n{}", all);
        assert!(all.contains("score"), "header missing:\n{}", all);
        assert!(all.contains("quit"), "hint bar missing:\n{}", all);
    }

    #[test]
    fn render_draws_the_action_menu_overlay() {
        let mut w = UserManagementWindow::new();
        let stats = stats_with_users();
        let colors = default_colors();
        let hotkeys = default_hotkeys();
        w.dialog = Some(UserDialog::Menu {
            uuid7: "u1".to_string(),
            username: "alice".to_string(),
        });

        let mut buf = Buffer::empty(Rect::new(0, 0, 80, 24));
        w.render(
            Rect::new(0, 0, 80, 24),
            &mut buf,
            true,
            &stats,
            &colors,
            &hotkeys,
            &[],
        );

        let mut all = String::new();
        for y in 0..24u16 {
            for x in 0..80u16 {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(
            all.contains("give reprimand"),
            "menu missing reprimand:\n{}",
            all
        );
        assert!(
            all.contains("give commendation"),
            "menu missing commendation:\n{}",
            all
        );
        assert!(
            all.contains("adjust score"),
            "menu missing adjust score:\n{}",
            all
        );
        assert!(
            all.contains("coming soon"),
            "menu missing link account:\n{}",
            all
        );
    }
}

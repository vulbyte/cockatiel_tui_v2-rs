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
    /// Candidate accounts for the link/merge overlay: every user EXCEPT the one
    /// the action menu was opened on. Captured when the menu opens because the
    /// dialog-key path has no access to `stats` (see [`Window::dialog_key`]).
    link_candidates: Vec<UserSummary>,
    /// Whether the account being linked is already merged (more than one
    /// channel), captured alongside the candidates. Gates the 3-option prompt.
    link_source_merged: bool,
}

/// The user-management modal. The `Menu` lists the per-user actions; the
/// `AdjustScore` arm is the small signed-integer editor opened by `s`; the two
/// `Link*` arms are the account-merge flow opened by `l` (pick a partner, then
/// confirm the merge).
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
    LinkPick(LinkPickState),
    LinkConfirm(LinkConfirmState),
}

/// Step 1 of the link/merge flow: a fuzzy search field plus the list of OTHER
/// accounts that can be linked to the selected user.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkPickState {
    /// The originally selected user (the one `l` was pressed on). Never offered
    /// as a candidate.
    source_uuid7: String,
    source_username: String,
    /// Fuzzy query over the candidate list.
    filter: String,
    /// Index into the filtered candidate list.
    selected: usize,
    /// Scroll offset for the candidate list.
    scroll: usize,
}

/// Which control of the confirm modal has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LinkFocus {
    Primary,
    Username,
    Roles,
    MergeChoice,
}

/// The operator's answer to the 3-option prompt shown when either account is
/// already merged (more than one channel).
///
/// Option 1 aborts; options 2/3 are mapped to the primary/secondary DIRECTION
/// (the wire payload has no field for the choice itself, so the choice only
/// picks which account survives — see [`LinkConfirmState::set_merge_choice`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MergeChoice {
    /// Option 1: nevermind — cancels the merge on confirm.
    Nevermind,
    /// Option 2: merge selected account with cockatiel account — the ORIGINAL
    /// selection survives (the default direction).
    MergeSelectedWithCockatiel,
    /// Option 3: merge both cockatiel accounts — the CANDIDATE survives.
    MergeBothCockatiel,
}

impl MergeChoice {
    /// Map the `1`/`2`/`3` keys to a choice.
    fn from_key(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::Nevermind),
            '2' => Some(Self::MergeSelectedWithCockatiel),
            '3' => Some(Self::MergeBothCockatiel),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Nevermind => "1) nevermind",
            Self::MergeSelectedWithCockatiel => "2) merge selected account with cockatiel account",
            Self::MergeBothCockatiel => "3) merge both cockatiel accounts",
        }
    }

    /// The three choices in display order.
    fn all() -> [Self; 3] {
        [
            Self::Nevermind,
            Self::MergeSelectedWithCockatiel,
            Self::MergeBothCockatiel,
        ]
    }
}

/// Step 2 of the link/merge flow: the confirm modal. Holds the two accounts,
/// which one survives, the editable resulting username, the roles winner, and
/// (only when either account is already merged) the 3-option choice.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LinkConfirmState {
    source_uuid7: String,
    source_username: String,
    candidate_uuid7: String,
    candidate_username: String,
    /// True when the ORIGINALLY SELECTED account survives (the default). The
    /// other account becomes the secondary (its channels move onto the
    /// survivor).
    primary_is_selected: bool,
    /// Editable resulting username, prefilled with the primary's username.
    username: String,
    /// True once the operator has edited `username`, so flipping the primary
    /// choice no longer re-prefills it.
    username_edited: bool,
    /// True when the SECONDARY's roles win.
    roles_from_secondary: bool,
    /// Present only when either account is already merged (more than one
    /// channel); `None` for a plain two-account merge.
    merge_choice: Option<MergeChoice>,
    /// Which control currently has keyboard focus.
    focus: LinkFocus,
}

impl LinkConfirmState {
    fn primary_uuid(&self) -> &str {
        if self.primary_is_selected {
            &self.source_uuid7
        } else {
            &self.candidate_uuid7
        }
    }

    fn secondary_uuid(&self) -> &str {
        if self.primary_is_selected {
            &self.candidate_uuid7
        } else {
            &self.source_uuid7
        }
    }

    fn primary_username(&self) -> &str {
        if self.primary_is_selected {
            &self.source_username
        } else {
            &self.candidate_username
        }
    }

    fn secondary_username(&self) -> &str {
        if self.primary_is_selected {
            &self.candidate_username
        } else {
            &self.source_username
        }
    }

    /// Flip which account survives, re-prefilling the username if the operator
    /// has not edited it.
    fn toggle_primary(&mut self) {
        self.primary_is_selected = !self.primary_is_selected;
        if !self.username_edited {
            self.username = self.primary_username().to_string();
        }
    }

    /// Record the 3-option choice. Options 2/3 also set the survival direction;
    /// option 1 is resolved (as an abort) at confirm time.
    fn set_merge_choice(&mut self, choice: MergeChoice) {
        self.merge_choice = Some(choice);
        match choice {
            MergeChoice::MergeSelectedWithCockatiel => self.primary_is_selected = true,
            MergeChoice::MergeBothCockatiel => self.primary_is_selected = false,
            MergeChoice::Nevermind => {}
        }
        if !self.username_edited {
            self.username = self.primary_username().to_string();
        }
    }

    /// Step the 3-option choice forward/back (Space cycles forward, Left/Right
    /// pick a direction), re-applying its survival direction.
    fn cycle_merge_choice(&mut self, forward: bool) {
        let all = MergeChoice::all();
        let current = self
            .merge_choice
            .and_then(|c| all.iter().position(|x| *x == c))
            .unwrap_or(1);
        let n = all.len() as i32;
        let next = if forward {
            (current as i32 + 1) % n
        } else {
            (current as i32 - 1 + n) % n
        };
        self.set_merge_choice(all[next as usize]);
    }

    /// The focusable controls in order, omitting the 3-option row when the
    /// prompt does not apply.
    fn focus_order(&self) -> Vec<LinkFocus> {
        let mut order = vec![LinkFocus::Primary, LinkFocus::Username, LinkFocus::Roles];
        if self.merge_choice.is_some() {
            order.push(LinkFocus::MergeChoice);
        }
        order
    }

    /// Move focus by `dir` (1 = forward, -1 = back), wrapping around.
    fn focus_step(&mut self, dir: i32) {
        let order = self.focus_order();
        let n = order.len() as i32;
        if n == 0 {
            return;
        }
        let i = order.iter().position(|f| *f == self.focus).unwrap_or(0) as i32;
        let j = ((i + dir) % n + n) % n;
        self.focus = order[j as usize];
    }
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
            link_candidates: Vec::new(),
            link_source_merged: false,
        }
    }

    // ── state helpers ───────────────────────────────────────────────

    /// The users matching the current fuzzy query, best score first. An empty
    /// query keeps the engine's order (score DESC).
    fn filtered_users<'a>(&self, users: &'a [UserSummary]) -> Vec<&'a UserSummary> {
        filter_users(&self.filter, users)
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
        // Capture the link candidates now: the dialog-key path cannot reach
        // `stats`, and the selected user must never be its own merge partner.
        self.link_candidates = link_candidates(&stats.users, &user.uuid7)
            .into_iter()
            .cloned()
            .collect();
        self.link_source_merged = is_merged(user);
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
                    // Step 1: pick the account to link (never the selected one).
                    self.status = None;
                    self.dialog = Some(UserDialog::LinkPick(LinkPickState {
                        source_uuid7: uuid7,
                        source_username: username,
                        filter: String::new(),
                        selected: 0,
                        scroll: 0,
                    }));
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
            UserDialog::LinkPick(pick) => self.link_pick_key(pick, key),
            UserDialog::LinkConfirm(confirm) => self.link_confirm_key(confirm, key),
        }
    }

    /// Resolve a key while the link/merge candidate picker is open. Printable
    /// keys type into the search field (this overlay is always in search mode),
    /// so navigation is arrows/PageUp/PageDown rather than `j`/`k`.
    fn link_pick_key(&mut self, mut pick: LinkPickState, key: KeyEvent) -> Option<Action> {
        let len = filtered_candidates(&self.link_candidates, &pick.filter).len();
        match key.code {
            KeyCode::Esc => {
                self.dialog = Some(UserDialog::Menu {
                    uuid7: pick.source_uuid7,
                    username: pick.source_username,
                });
                Some(Action::Noop)
            }
            KeyCode::Down => {
                pick.selected = (pick.selected + 1).min(len.saturating_sub(1));
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
            KeyCode::Up => {
                pick.selected = pick.selected.saturating_sub(1);
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
            KeyCode::PageDown => {
                pick.selected = (pick.selected + 10).min(len.saturating_sub(1));
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
            KeyCode::PageUp => {
                pick.selected = pick.selected.saturating_sub(10);
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
            KeyCode::Backspace => {
                pick.filter.pop();
                pick.selected = 0;
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
            KeyCode::Char(c)
                if key.kind != KeyEventKind::Release
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                pick.filter.push(c);
                pick.selected = 0;
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
            KeyCode::Enter => {
                let list = filtered_candidates(&self.link_candidates, &pick.filter);
                let Some(candidate) = list.get(pick.selected) else {
                    self.dialog = Some(UserDialog::LinkPick(pick));
                    return Some(Action::Noop);
                };
                // The 3-option prompt appears when EITHER account is merged.
                let merged = self.link_source_merged || is_merged(candidate);
                let source_username = pick.source_username.clone();
                self.dialog = Some(UserDialog::LinkConfirm(LinkConfirmState {
                    source_uuid7: pick.source_uuid7,
                    source_username: pick.source_username,
                    candidate_uuid7: candidate.uuid7.clone(),
                    candidate_username: candidate.username.clone(),
                    primary_is_selected: true,
                    username: source_username,
                    username_edited: false,
                    roles_from_secondary: false,
                    merge_choice: merged.then_some(MergeChoice::MergeSelectedWithCockatiel),
                    focus: LinkFocus::Primary,
                }));
                Some(Action::Noop)
            }
            _ => {
                self.dialog = Some(UserDialog::LinkPick(pick));
                Some(Action::Noop)
            }
        }
    }

    /// Resolve a key while the merge confirm modal is open. `Enter` emits the
    /// `userdb_merge_users` query (or cancels on the `nevermind` choice);
    /// everything else only changes the modal's own state.
    fn link_confirm_key(&mut self, mut confirm: LinkConfirmState, key: KeyEvent) -> Option<Action> {
        match key.code {
            KeyCode::Esc => {
                // Back to the picker rather than all the way out.
                self.dialog = Some(UserDialog::LinkPick(LinkPickState {
                    source_uuid7: confirm.source_uuid7,
                    source_username: confirm.source_username,
                    filter: String::new(),
                    selected: 0,
                    scroll: 0,
                }));
                Some(Action::Noop)
            }
            KeyCode::Tab | KeyCode::Down => {
                confirm.focus_step(1);
                self.dialog = Some(UserDialog::LinkConfirm(confirm));
                Some(Action::Noop)
            }
            KeyCode::BackTab | KeyCode::Up => {
                confirm.focus_step(-1);
                self.dialog = Some(UserDialog::LinkConfirm(confirm));
                Some(Action::Noop)
            }
            KeyCode::Enter => self.confirm_merge(confirm),
            KeyCode::Backspace if confirm.focus == LinkFocus::Username => {
                confirm.username.pop();
                confirm.username_edited = true;
                self.dialog = Some(UserDialog::LinkConfirm(confirm));
                Some(Action::Noop)
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Char(' ')
                if key.kind != KeyEventKind::Release =>
            {
                match confirm.focus {
                    LinkFocus::Primary => confirm.toggle_primary(),
                    LinkFocus::Roles => {
                        confirm.roles_from_secondary = !confirm.roles_from_secondary
                    }
                    LinkFocus::MergeChoice => {
                        confirm.cycle_merge_choice(key.code == KeyCode::Right)
                    }
                    LinkFocus::Username => {
                        // Space is ordinary text in the username field.
                        if key.code == KeyCode::Char(' ') {
                            confirm.username.push(' ');
                            confirm.username_edited = true;
                        }
                    }
                }
                self.dialog = Some(UserDialog::LinkConfirm(confirm));
                Some(Action::Noop)
            }
            KeyCode::Char(c)
                if key.kind != KeyEventKind::Release
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                match confirm.focus {
                    LinkFocus::Username => {
                        confirm.username.push(c);
                        confirm.username_edited = true;
                    }
                    LinkFocus::MergeChoice => {
                        if let Some(choice) = MergeChoice::from_key(c) {
                            confirm.set_merge_choice(choice);
                        }
                    }
                    LinkFocus::Primary | LinkFocus::Roles => {}
                }
                self.dialog = Some(UserDialog::LinkConfirm(confirm));
                Some(Action::Noop)
            }
            _ => {
                self.dialog = Some(UserDialog::LinkConfirm(confirm));
                Some(Action::Noop)
            }
        }
    }

    /// Resolve the confirm modal: emit the merge query, or cancel on the
    /// `nevermind` choice. An empty (or whitespace) username falls back to the
    /// surviving account's username so the engine never renames a user to "".
    fn confirm_merge(&mut self, confirm: LinkConfirmState) -> Option<Action> {
        if confirm.merge_choice == Some(MergeChoice::Nevermind) {
            self.status = Some(format!(
                "merge cancelled for {} and {}",
                confirm.source_username, confirm.candidate_username
            ));
            return Some(Action::Noop);
        }
        let username = if confirm.username.trim().is_empty() {
            confirm.primary_username().to_string()
        } else {
            confirm.username.clone()
        };
        self.status = Some(format!(
            "merging {} into {} as {} (roles from {})",
            confirm.secondary_username(),
            confirm.primary_username(),
            username,
            if confirm.roles_from_secondary {
                "secondary"
            } else {
                "primary"
            }
        ));
        Some(merge_query(
            confirm.primary_uuid(),
            confirm.secondary_uuid(),
            &username,
            confirm.roles_from_secondary,
        ))
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

/// The engine virtual query that merges two user accounts (the TUI's link
/// action). The backend honours this payload exactly; see the link/merge flow
/// in this window.
pub(crate) const MERGE_QUERY_ID: &str = "userdb_merge_users";

/// Build the one-shot merge query. `username` is passed through verbatim (the
/// confirm modal decides the empty-username fallback), and
/// `roles_from_secondary` selects which account's roles win.
pub(crate) fn merge_query(
    primary_uuid7: &str,
    secondary_uuid7: &str,
    username: &str,
    roles_from_secondary: bool,
) -> Action {
    let payload = serde_json::json!({
        "primary_uuid7": primary_uuid7,
        "secondary_uuid7": secondary_uuid7,
        "username": username,
        "roles_from_secondary": roles_from_secondary,
    });
    Action::UserQuery(MERGE_QUERY_ID.to_string(), payload.to_string())
}

/// Whether an account is already merged (more than one channel). Used to decide
/// whether the link flow shows the 3-option prompt.
pub(crate) fn is_merged(user: &UserSummary) -> bool {
    user.channels.len() > 1
}

/// Every account that may be linked to `exclude_uuid7`: all users EXCEPT the
/// one being linked. The selected user must never be offered as its own merge
/// partner.
pub(crate) fn link_candidates<'a>(
    users: &'a [UserSummary],
    exclude_uuid7: &str,
) -> Vec<&'a UserSummary> {
    users.iter().filter(|u| u.uuid7 != exclude_uuid7).collect()
}

/// Filter users by a fuzzy query, best score first. An empty query keeps the
/// input order (the engine's score DESC). Shared by the window's own list and
/// the link/merge candidate picker.
pub(crate) fn filter_users<'a>(query: &str, users: &'a [UserSummary]) -> Vec<&'a UserSummary> {
    if query.trim().is_empty() {
        return users.iter().collect();
    }
    let mut scored: Vec<(usize, &UserSummary)> = users
        .iter()
        .filter_map(|u| {
            let score = fuzzy_match_user(query, u);
            (score > 0).then_some((score, u))
        })
        .collect();
    // Stable sort: equal scores keep the input order.
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored.into_iter().map(|(_, u)| u).collect()
}

/// The filtered candidate list for the link picker.
fn filtered_candidates<'a>(candidates: &'a [UserSummary], query: &str) -> Vec<&'a UserSummary> {
    filter_users(query, candidates)
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
fn render_dialog(
    dialog: &Option<UserDialog>,
    candidates: &[UserSummary],
    area: Rect,
    buf: &mut Buffer,
    colors: &ColorConfig,
) {
    let Some(dialog) = dialog else { return };
    match dialog {
        UserDialog::LinkPick(pick) => render_link_pick(pick, candidates, area, buf, colors),
        UserDialog::LinkConfirm(confirm) => render_link_confirm(confirm, area, buf, colors),
        UserDialog::Menu { .. } | UserDialog::AdjustScore { .. } => {
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
                            " [l] link account (merge)".to_string(),
                            Style::default().fg(Color::Cyan),
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
                _ => unreachable!(),
            };
            draw_modal(title, &lines, area, buf, colors, 56);
        }
    }
}

/// Draw a centered modal box with a title and pre-styled lines, clearing the
/// region first so the list never shows through. Returns the inner rect.
fn draw_modal(
    title: &str,
    lines: &[(String, Style)],
    area: Rect,
    buf: &mut Buffer,
    colors: &ColorConfig,
    max_width: u16,
) -> Rect {
    let w = max_width.min(area.width.saturating_sub(2)).max(24);
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
    inner
}

/// Step 1 overlay: a search field plus the list of other accounts that can be
/// linked. Already-merged candidates are flagged so the operator sees which
/// picks will trigger the 3-option prompt.
fn render_link_pick(
    pick: &LinkPickState,
    candidates: &[UserSummary],
    area: Rect,
    buf: &mut Buffer,
    colors: &ColorConfig,
) {
    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let list = filtered_candidates(candidates, &pick.filter);
    let budget = area.height.saturating_sub(8).clamp(1, 12) as usize;
    let visible = list.len().min(budget);
    let mut scroll = pick.scroll.min(list.len().saturating_sub(visible));
    if pick.selected < scroll {
        scroll = pick.selected;
    } else if visible > 0 && pick.selected >= scroll + visible {
        scroll = pick.selected + 1 - visible;
    }

    let mut lines: Vec<(String, Style)> = Vec::new();
    lines.push((
        format!(" Link a second account to {}:", pick.source_username),
        bold,
    ));
    lines.push((
        format!(" search: {}", pick.filter),
        Style::default().fg(Color::Yellow),
    ));
    lines.push((String::new(), Style::default()));
    if list.is_empty() {
        lines.push((
            "  (no other accounts match)".to_string(),
            Style::default().fg(Color::DarkGray),
        ));
    } else {
        for (i, user) in list.iter().enumerate().skip(scroll).take(visible) {
            let selected = i == pick.selected;
            let merged = is_merged(user);
            let mut text = format!("  {}", display_name(user));
            if merged {
                text.push_str("  (already merged)");
            }
            let style = if selected {
                row_style(true)
            } else if merged {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::White)
            };
            lines.push((truncate(&text, 68), style));
        }
    }
    lines.push((String::new(), Style::default()));
    lines.push((
        " type to search   up/down pick   enter select   esc back".to_string(),
        Style::default().fg(Color::DarkGray),
    ));
    draw_modal(" link account ", &lines, area, buf, colors, 72);
}

/// Step 2 overlay: confirm the merge. Shows the survivor choice, the editable
/// resulting username, the roles winner and — when either account is already
/// merged — the 3-option choice.
fn render_link_confirm(
    confirm: &LinkConfirmState,
    area: Rect,
    buf: &mut Buffer,
    colors: &ColorConfig,
) {
    let bold = Style::default()
        .fg(Color::White)
        .add_modifier(Modifier::BOLD);
    let focus_style = Style::default().fg(Color::Black).bg(Color::LightGreen);
    let normal = Style::default().fg(Color::White);
    let dim = Style::default().fg(Color::DarkGray);
    let mark = |f: LinkFocus| if confirm.focus == f { "▸" } else { " " };

    let mut lines: Vec<(String, Style)> = Vec::new();
    lines.push((
        format!(
            " Link {} with {}",
            confirm.source_username, confirm.candidate_username
        ),
        bold,
    ));
    lines.push((String::new(), Style::default()));

    // Which account survives (the primary).
    let (s_mark, c_mark) = if confirm.primary_is_selected {
        ("[x]", "[ ]")
    } else {
        ("[ ]", "[x]")
    };
    lines.push((
        format!(
            "{} Survivor: {} {}   {} {}",
            mark(LinkFocus::Primary),
            s_mark,
            confirm.source_username,
            c_mark,
            confirm.candidate_username
        ),
        if confirm.focus == LinkFocus::Primary {
            focus_style
        } else {
            normal
        },
    ));

    // Editable resulting username.
    let cursor = if confirm.focus == LinkFocus::Username {
        "▏"
    } else {
        ""
    };
    lines.push((
        format!(
            "{} Username: {}{}",
            mark(LinkFocus::Username),
            confirm.username,
            cursor
        ),
        if confirm.focus == LinkFocus::Username {
            focus_style
        } else {
            normal
        },
    ));

    // Roles winner.
    let roles = if confirm.roles_from_secondary {
        "secondary"
    } else {
        "primary"
    };
    lines.push((
        format!(
            "{} Roles winner: {} (space toggles)",
            mark(LinkFocus::Roles),
            roles
        ),
        if confirm.focus == LinkFocus::Roles {
            focus_style
        } else {
            normal
        },
    ));

    // The 3-option prompt, only when an account is already merged.
    if let Some(choice) = confirm.merge_choice {
        lines.push((String::new(), Style::default()));
        lines.push((
            " Already merged — choose:".to_string(),
            Style::default().fg(Color::Yellow),
        ));
        for option in MergeChoice::all() {
            let selected = option == choice;
            let marker = if selected { "●" } else { "○" };
            let style = if selected && confirm.focus == LinkFocus::MergeChoice {
                focus_style
            } else if selected {
                Style::default().fg(Color::Cyan)
            } else {
                dim
            };
            lines.push((format!("   {} {}", marker, option.label()), style));
        }
    }

    lines.push((String::new(), Style::default()));
    lines.push((
        " tab focus   space toggle   1/2/3 choose   enter merge   esc back".to_string(),
        dim,
    ));
    draw_modal(" confirm merge ", &lines, area, buf, colors, 72);
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

        let hints = match &self.dialog {
            Some(UserDialog::Menu { .. }) => "menu:[r|c|s|l]  cancel:[esc]".to_string(),
            Some(UserDialog::AdjustScore { .. }) => {
                "type a signed integer  apply:[enter]  cancel:[esc]".to_string()
            }
            Some(UserDialog::LinkPick(_)) => {
                "search:[type]  pick:[up|down]  select:[enter]  back:[esc]".to_string()
            }
            Some(UserDialog::LinkConfirm(_)) => {
                "focus:[tab]  toggle:[space]  merge:[enter]  back:[esc]".to_string()
            }
            None => "nav:[j|k|g|G]  search:[/]  select:[enter]  quit:[q|esc]".to_string(),
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
                let style = if status.contains("coming soon")
                    || status.contains("must be")
                    || status.contains("cancelled")
                {
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
        render_dialog(&self.dialog, &self.link_candidates, area, buf, colors);
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
    fn link_pick_excludes_the_selected_user_and_emits_a_merge() {
        let mut w = UserManagementWindow::new();
        let mut stats = stats_with_users();

        // Enter opens the action menu on the first user (alice).
        assert_eq!(
            w.handle_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()),
                &mut stats
            ),
            Some(Action::Noop)
        );
        assert_eq!(
            w.link_candidates
                .iter()
                .map(|u| u.uuid7.as_str())
                .collect::<Vec<_>>(),
            vec!["u2", "u3", "u4"],
            "the selected user must never be offered as its own merge partner"
        );

        // `l` opens the candidate picker; the source is captured.
        assert_eq!(w.dialog_key(key('l')), Some(Action::Noop));
        match &w.dialog {
            Some(UserDialog::LinkPick(p)) => {
                assert_eq!(p.source_uuid7, "u1");
                assert_eq!(p.source_username, "alice");
            }
            other => panic!("expected LinkPick, got {:?}", other),
        }

        // Enter picks the first candidate (bob) and opens the confirm modal.
        assert_eq!(
            w.dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            Some(Action::Noop)
        );
        match &w.dialog {
            Some(UserDialog::LinkConfirm(c)) => {
                assert_eq!(c.candidate_uuid7, "u2");
                assert_eq!(c.candidate_username, "bob");
                assert!(
                    c.primary_is_selected,
                    "the original selection survives by default"
                );
                assert_eq!(c.username, "alice", "prefilled with the primary's username");
            }
            other => panic!("expected LinkConfirm, got {:?}", other),
        }

        // Enter confirms and emits the merge payload.
        let action = w
            .dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        match action {
            Action::UserQuery(id, payload) => {
                assert_eq!(id, "userdb_merge_users");
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["primary_uuid7"], "u1");
                assert_eq!(v["secondary_uuid7"], "u2");
                assert_eq!(v["username"], "alice");
                assert_eq!(v["roles_from_secondary"], false);
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
        assert!(!w.in_dialog());
        assert!(w.status.as_deref().unwrap().contains("merging"));
    }

    #[test]
    fn merge_query_carries_every_field() {
        let action = merge_query("p1", "s1", "new_name", true);
        match action {
            Action::UserQuery(id, payload) => {
                assert_eq!(id, "userdb_merge_users");
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["primary_uuid7"], "p1");
                assert_eq!(v["secondary_uuid7"], "s1");
                assert_eq!(v["username"], "new_name");
                assert_eq!(v["roles_from_secondary"], true);
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
    }

    #[test]
    fn merge_query_passes_an_empty_username_through() {
        match merge_query("p1", "s1", "", false) {
            Action::UserQuery(id, payload) => {
                assert_eq!(id, "userdb_merge_users");
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["username"], "");
                assert_eq!(v["roles_from_secondary"], false);
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
    }

    #[test]
    fn link_candidates_never_include_the_excluded_user() {
        let stats = stats_with_users();
        let candidates = link_candidates(&stats.users, "u3");
        assert!(candidates.iter().all(|u| u.uuid7 != "u3"));
        assert_eq!(candidates.len(), 3);
        assert!(is_merged(&user("x", "x", 0, 0.0, &["a:1 (a)", "b:2 (b)"])));
        assert!(!is_merged(&user("y", "y", 0, 0.0, &["a:1 (a)"])));
    }

    #[test]
    fn cleared_username_falls_back_to_the_primary() {
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::LinkConfirm(LinkConfirmState {
            source_uuid7: "u1".into(),
            source_username: "alice".into(),
            candidate_uuid7: "u2".into(),
            candidate_username: "bob".into(),
            primary_is_selected: true,
            username: "   ".into(),
            username_edited: true,
            roles_from_secondary: false,
            merge_choice: None,
            focus: LinkFocus::Username,
        }));
        let action = w
            .dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap();
        match action {
            Action::UserQuery(_, payload) => {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(
                    v["username"], "alice",
                    "empty input falls back to the survivor"
                );
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
    }

    #[test]
    fn merge_choice_plumbs_nevermind_and_direction() {
        let base = || LinkConfirmState {
            source_uuid7: "u1".into(),
            source_username: "alice".into(),
            candidate_uuid7: "u2".into(),
            candidate_username: "bob".into(),
            primary_is_selected: true,
            username: "alice".into(),
            username_edited: false,
            roles_from_secondary: false,
            merge_choice: Some(MergeChoice::MergeSelectedWithCockatiel),
            focus: LinkFocus::MergeChoice,
        };

        // Option 1 (nevermind): confirm emits nothing and cancels.
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::LinkConfirm(base()));
        assert_eq!(w.dialog_key(key('1')), Some(Action::Noop));
        assert_eq!(
            w.dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty())),
            Some(Action::Noop)
        );
        assert!(w.status.as_deref().unwrap().contains("cancelled"));

        // Option 2: the original selection stays primary.
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::LinkConfirm(base()));
        w.dialog_key(key('2'));
        match w
            .dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap()
        {
            Action::UserQuery(_, payload) => {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["primary_uuid7"], "u1");
                assert_eq!(v["secondary_uuid7"], "u2");
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }

        // Option 3: the candidate becomes primary (direction reverses).
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::LinkConfirm(base()));
        w.dialog_key(key('3'));
        match w
            .dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap()
        {
            Action::UserQuery(_, payload) => {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["primary_uuid7"], "u2");
                assert_eq!(v["secondary_uuid7"], "u1");
                assert_eq!(
                    v["username"], "bob",
                    "the new primary re-prefills the username"
                );
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
    }

    #[test]
    fn roles_winner_toggle_sets_the_payload_flag() {
        let mut w = UserManagementWindow::new();
        w.dialog = Some(UserDialog::LinkConfirm(LinkConfirmState {
            source_uuid7: "u1".into(),
            source_username: "alice".into(),
            candidate_uuid7: "u2".into(),
            candidate_username: "bob".into(),
            primary_is_selected: true,
            username: "alice".into(),
            username_edited: false,
            roles_from_secondary: false,
            merge_choice: None,
            focus: LinkFocus::Roles,
        }));
        // Space toggles the roles winner while the row has focus.
        assert_eq!(w.dialog_key(key(' ')), Some(Action::Noop));
        match w
            .dialog_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::empty()))
            .unwrap()
        {
            Action::UserQuery(_, payload) => {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                assert_eq!(v["roles_from_secondary"], true);
            }
            other => panic!("expected UserQuery, got {:?}", other),
        }
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
            all.contains("link account"),
            "menu missing link account:\n{}",
            all
        );
    }

    #[test]
    fn render_draws_the_merge_confirm_with_the_three_options() {
        let mut w = UserManagementWindow::new();
        let stats = stats_with_users();
        let colors = default_colors();
        let hotkeys = default_hotkeys();
        w.dialog = Some(UserDialog::LinkConfirm(LinkConfirmState {
            source_uuid7: "u1".into(),
            source_username: "alice".into(),
            candidate_uuid7: "u3".into(),
            candidate_username: "carol".into(),
            primary_is_selected: true,
            username: "alice".into(),
            username_edited: false,
            roles_from_secondary: false,
            merge_choice: Some(MergeChoice::MergeSelectedWithCockatiel),
            focus: LinkFocus::Primary,
        }));

        let mut buf = Buffer::empty(Rect::new(0, 0, 100, 30));
        w.render(
            Rect::new(0, 0, 100, 30),
            &mut buf,
            true,
            &stats,
            &colors,
            &hotkeys,
            &[],
        );

        let mut all = String::new();
        for y in 0..30u16 {
            for x in 0..100u16 {
                all.push_str(buf[(x, y)].symbol());
            }
            all.push('\n');
        }
        assert!(all.contains("confirm merge"), "title missing:\n{}", all);
        assert!(all.contains("Survivor"), "survivor row missing:\n{}", all);
        assert!(all.contains("Roles winner"), "roles row missing:\n{}", all);
        assert!(
            all.contains("nevermind"),
            "3-option prompt missing:\n{}",
            all
        );
        assert!(
            all.contains("merge both cockatiel accounts"),
            "option 3 missing:\n{}",
            all
        );
    }
}

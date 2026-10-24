use std::collections::HashMap;
use std::path::Path;

use ratatui::style::Color;
use serde::Deserialize;

/// The legacy color file (`color_config.json`): the three per-window /
/// per-status / per-platform colour maps at the TOP level.
#[derive(Debug, Deserialize, Default)]
struct ColorFile {
    #[serde(default)]
    borders: HashMap<String, String>,
    #[serde(default)]
    status: HashMap<String, String>,
    #[serde(default)]
    platforms: HashMap<String, String>,
}

/// The `colorscheme` object inside the TUI's own `config.json`.
///
/// Same three maps as the legacy file, plus the scalar accent keys. Every
/// scalar is held as a `String` (not a `Color`) so the blank-vs-set rule can be
/// applied at merge time: a blank value means "use the built-in default" (see
/// [`default_colors`]), a set value is parsed with [`parse_color`].
#[derive(Debug, Deserialize, Default)]
struct ColorScheme {
    #[serde(default)]
    primary: String,
    #[serde(default)]
    secondary: String,
    #[serde(default)]
    background: String,
    #[serde(default, rename = "active-border", alias = "active_border")]
    active_border: String,
    #[serde(default, rename = "inactive-border", alias = "inactive_border")]
    inactive_border: String,
    #[serde(default)]
    borders: HashMap<String, String>,
    #[serde(default)]
    status: HashMap<String, String>,
    #[serde(default)]
    platforms: HashMap<String, String>,
}

/// The slice of the TUI's `config.json` that carries colours. Unknown keys are
/// ignored, so this never fights the rest of the TUI config.
#[derive(Debug, Deserialize, Default)]
struct TuiConfigColorFile {
    #[serde(default)]
    colorscheme: ColorScheme,
}

#[derive(Debug, Clone)]
pub struct ColorConfig {
    pub borders: HashMap<String, Color>,
    pub status: HashMap<String, Color>,
    pub platforms: HashMap<String, Color>,
    /// The app's primary accent (active chrome such as the focused pane
    /// header). Set by `colorscheme.primary`; blank leaves the built-in default.
    pub primary: Color,
    /// The app's secondary accent (section headers). Set by
    /// `colorscheme.secondary`; blank leaves the built-in default.
    pub secondary: Color,
    /// The app / terminal background. `Color::Reset` (the default) leaves the
    /// terminal's own background in place; a set value paints the root.
    pub background: Color,
    /// An explicit `colorscheme.active-border`. When set it overrides the
    /// per-window name-derived hue; when `None` (blank) the active border is
    /// derived from the window name (see [`ColorConfig::active_border_color`]).
    pub active_border: Option<Color>,
    /// An explicit `colorscheme.inactive-border`. When set it is THE unfocused
    /// window border; when `None` (blank) the `borders` map decides (falling
    /// back to [`Color::DarkGray`]).
    pub inactive_border: Option<Color>,
}

fn parse_color(s: &str) -> Color {
    let s = s.trim();
    // `#rrggbb` truecolor (terminals without truecolor will approximate it).
    if let Some(hex) = s.strip_prefix('#') {
        if hex.len() == 6 && hex.is_ascii() {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Color::Rgb(r, g, b);
            }
        }
    }
    match s.to_lowercase().as_str() {
        "black" => Color::Black,
        "red" => Color::Red,
        "green" => Color::Green,
        "yellow" => Color::Yellow,
        "blue" => Color::Blue,
        "magenta" | "purple" => Color::Magenta,
        "cyan" => Color::Cyan,
        "white" => Color::White,
        "gray" | "grey" => Color::Gray,
        "darkgray" | "darkgrey" => Color::DarkGray,
        "lightred" => Color::LightRed,
        "lightgreen" => Color::LightGreen,
        "lightyellow" => Color::LightYellow,
        "lightblue" => Color::LightBlue,
        "lightmagenta" => Color::LightMagenta,
        "lightcyan" => Color::LightCyan,
        "orange" => Color::Indexed(208),
        "pink" => Color::Indexed(205),
        "brown" => Color::Indexed(130),
        _ => {
            if let Ok(n) = s.parse::<u8>() {
                Color::Indexed(n)
            } else {
                Color::White
            }
        }
    }
}

/// Parse a colour value, treating a blank (`""`, or whitespace) as "not set".
/// This is the blank-vs-set rule: a blank value falls back to the built-in
/// default rather than being parsed into some accidental colour.
fn parse_opt(s: &str) -> Option<Color> {
    if s.trim().is_empty() {
        None
    } else {
        Some(parse_color(s))
    }
}

/// Merge a set of string colour maps over `colors`. Blank values are skipped —
/// absence in the map (or a blank entry) means "keep the built-in default".
fn apply_maps(
    colors: &mut ColorConfig,
    borders: &HashMap<String, String>,
    status: &HashMap<String, String>,
    platforms: &HashMap<String, String>,
) {
    for (k, v) in borders {
        if let Some(c) = parse_opt(v) {
            colors.borders.insert(k.clone(), c);
        }
    }
    for (k, v) in status {
        if let Some(c) = parse_opt(v) {
            colors.status.insert(k.clone(), c);
        }
    }
    for (k, v) in platforms {
        if let Some(c) = parse_opt(v) {
            colors.platforms.insert(k.clone(), c);
        }
    }
}

/// Load the colour config for the TUI.
///
/// `config_path` is the TUI's OWN `config.json`. Colours come from two places,
/// merged in this order over the built-in [`default_colors`]:
///
/// 1. the **legacy** `color_config.json` sitting beside it (so existing configs
///    keep working), then
/// 2. the `colorscheme` object in `config.json` itself, which WINS.
///
/// Merging (rather than replacing) means a config that omits a key — or
/// predates a newly-added status like `building` — still gets a sensible
/// default instead of falling back to white.
pub fn load_colors(config_path: &Path) -> ColorConfig {
    let mut colors = default_colors();

    // 1. Legacy `color_config.json` in the same directory. A bare/empty path
    //    (used by tests) has no directory, so it reads only defaults.
    if let Some(dir) = config_path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let legacy_path = dir.join("color_config.json");
        if let Ok(content) = std::fs::read_to_string(&legacy_path) {
            if let Ok(file) = serde_json::from_str::<ColorFile>(&content) {
                apply_maps(&mut colors, &file.borders, &file.status, &file.platforms);
            }
        }
    }

    // 2. The new `colorscheme` object in the TUI's config.json — applied last,
    //    so it wins over the legacy file.
    if let Ok(content) = std::fs::read_to_string(config_path) {
        if let Ok(file) = serde_json::from_str::<TuiConfigColorFile>(&content) {
            let cs = file.colorscheme;
            apply_maps(&mut colors, &cs.borders, &cs.status, &cs.platforms);
            if let Some(c) = parse_opt(&cs.primary) {
                colors.primary = c;
            }
            if let Some(c) = parse_opt(&cs.secondary) {
                colors.secondary = c;
            }
            if let Some(c) = parse_opt(&cs.background) {
                colors.background = c;
            }
            colors.active_border = parse_opt(&cs.active_border);
            colors.inactive_border = parse_opt(&cs.inactive_border);
        }
    }
    colors
}

pub fn default_colors() -> ColorConfig {
    let mut borders = HashMap::new();
    borders.insert("inactive".to_string(), Color::DarkGray);
    borders.insert("logo".to_string(), Color::Indexed(208));
    borders.insert("log".to_string(), Color::Yellow);
    borders.insert("db_platforms".to_string(), Color::Magenta);
    borders.insert("modules".to_string(), Color::Cyan);
    borders.insert("hotkey_bar".to_string(), Color::Green);
    borders.insert("chart".to_string(), Color::Blue);
    borders.insert("prompts".to_string(), Color::Magenta);
    borders.insert("users".to_string(), Color::LightCyan);
    borders.insert("stream_manager".to_string(), Color::LightGreen);
    borders.insert("user_management".to_string(), Color::LightMagenta);

    let mut status = HashMap::new();
    status.insert("online".to_string(), Color::Green);
    status.insert("connected".to_string(), Color::Green);
    status.insert("starting".to_string(), Color::Yellow);
    // A module mid-build: a muted dark pastel blue (#779ecb) so it reads as
    // "working, not ready yet" and is distinct from the yellow lifecycle
    // states around it.
    status.insert("building".to_string(), Color::Rgb(119, 158, 203));
    status.insert("restarting".to_string(), Color::Yellow);
    status.insert("offline".to_string(), Color::DarkGray);
    status.insert("stopped".to_string(), Color::Yellow);
    status.insert("disconnected".to_string(), Color::Yellow);
    status.insert("error".to_string(), Color::LightRed);
    status.insert("crashed".to_string(), Color::Red);

    let mut platforms = HashMap::new();
    platforms.insert("twitch".to_string(), Color::Magenta);
    platforms.insert("youtube".to_string(), Color::Red);
    platforms.insert("kick".to_string(), Color::Green);
    platforms.insert("discord".to_string(), Color::Blue);

    ColorConfig {
        borders,
        status,
        platforms,
        // The app's accent pair: the active chrome is cyan, section headers are
        // magenta. Both are the colours the UI already used before they became
        // configurable, so a blank `colorscheme` changes nothing.
        primary: Color::Cyan,
        secondary: Color::Magenta,
        // `Reset` = leave the terminal's own background alone.
        background: Color::Reset,
        active_border: None,
        inactive_border: None,
    }
}

/// The HSV hue (degrees, 0-360) for a window name's first ASCII letter:
/// `a`→0° (red) … `z`→300° (magenta), evenly spaced across a–z. `None` when the
/// first character is not an ASCII letter.
fn hue_from_name(name: &str) -> Option<f32> {
    let c = name.chars().next()?;
    if !c.is_ascii_alphabetic() {
        return None;
    }
    let idx = (c.to_ascii_lowercase() as u8 - b'a') as f32;
    Some(idx / 25.0 * 300.0)
}

/// HSV(h, 1.0, 1.0) → RGB, with `h` in degrees. The full-saturation / full-value
/// case only needs the hue sector and the second-largest channel.
fn hsv_to_rgb(h: f32) -> (u8, u8, u8) {
    let h = h.rem_euclid(360.0);
    let c = 1.0_f32;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let to_u8 = |v: f32| (v * 255.0).round().clamp(0.0, 255.0) as u8;
    (to_u8(r), to_u8(g), to_u8(b))
}

impl ColorConfig {
    /// The UNFOCUSED border colour for a window.
    ///
    /// An explicit `colorscheme.inactive-border` is the operator's chosen
    /// unfocused border and wins; otherwise the window's `borders` entry is
    /// used, falling back to dark gray when the window has no entry.
    pub fn border_color(&self, window_name: &str) -> Color {
        match self.borders.get(window_name) {
            // The `inactive` pseudo-window IS "the default unfocused border",
            // so an explicit `inactive-border` replaces it rather than being
            // ignored.
            Some(c) if window_name == "inactive" => self.inactive_border.unwrap_or(*c),
            Some(c) => *c,
            None => self.inactive_border.unwrap_or(Color::DarkGray),
        }
    }

    /// The FOCUSED border colour for a window.
    ///
    /// An explicit `colorscheme.active-border` wins outright. Otherwise the
    /// colour is a hue derived from the window's NAME: the first ASCII letter
    /// maps `a`→red (0°) … `z`→magenta (300°), so each window has its own
    /// stable, recognisable focus colour. A name whose first char is not an
    /// ASCII letter falls back to gray.
    pub fn active_border_color(&self, window_name: &str) -> Color {
        if let Some(c) = self.active_border {
            return c;
        }
        match hue_from_name(window_name) {
            Some(h) => {
                let (r, g, b) = hsv_to_rgb(h);
                Color::Rgb(r, g, b)
            }
            None => Color::Gray,
        }
    }

    pub fn status_color(&self, status: &str) -> Color {
        if let Some(c) = self.status.get(status) {
            *c
        } else {
            Color::White
        }
    }

    pub fn platform_color(&self, platform: &str) -> Color {
        if let Some(c) = self.platforms.get(platform) {
            *c
        } else {
            Color::White
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_config_path(name: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("ckt-colors-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        (dir.clone(), dir.join(name))
    }

    #[test]
    fn parse_color_supports_hex_and_named() {
        assert_eq!(parse_color("#779ecb"), Color::Rgb(119, 158, 203));
        assert_eq!(parse_color("#FFFFFF"), Color::Rgb(255, 255, 255));
        assert_eq!(parse_color(" cyan "), Color::Cyan);
        // A malformed value falls back to white rather than panicking.
        assert_eq!(parse_color("not-a-color"), Color::White);
        assert_eq!(parse_color("#zzzzzz"), Color::White);
    }

    #[test]
    fn active_border_hue_follows_the_first_letter() {
        let colors = default_colors();
        assert_eq!(
            colors.active_border_color("a"),
            Color::Rgb(255, 0, 0),
            "a must map to red (hue 0)"
        );
        assert_eq!(
            colors.active_border_color("z"),
            Color::Rgb(255, 0, 255),
            "z must map to magenta (hue 300)"
        );
        let mid = colors.active_border_color("modules");
        match mid {
            Color::Rgb(r, g, b) => {
                assert!(
                    r < 255 && g > 0 && b > 0 && b < 255,
                    "a middle letter must land strictly between red and magenta: {mid:?}"
                );
            }
            other => panic!("a letter must produce an Rgb hue, got {other:?}"),
        }
        assert_eq!(
            colors.active_border_color("_engine"),
            Color::Gray,
            "a non-letter first char falls back to gray"
        );
        // b is one step in (1/25 of the way): ~12 degrees, a red-orange.
        assert_eq!(colors.active_border_color("b"), Color::Rgb(255, 51, 0));
    }

    #[test]
    fn explicit_active_border_overrides_the_name_hue() {
        let mut colors = default_colors();
        colors.active_border = Some(Color::Blue);
        assert_eq!(colors.active_border_color("a"), Color::Blue);
    }

    #[test]
    fn explicit_inactive_border_overrides_the_borders_map() {
        let mut colors = default_colors();
        colors.inactive_border = Some(Color::Yellow);
        assert_eq!(colors.border_color("inactive"), Color::Yellow);
        assert_eq!(colors.border_color("not-a-window"), Color::Yellow);
    }

    #[test]
    fn colorscheme_blank_means_default_and_set_means_custom() {
        let (dir, path) = temp_config_path("config.json");
        // Blank scalars and empty maps: pure defaults.
        std::fs::write(
            &path,
            r#"{"colorscheme":{"primary":"","secondary":"","background":"","active-border":"","inactive-border":"","borders":{},"status":{},"platforms":{}}}"#,
        )
        .unwrap();
        let colors = load_colors(&path);
        let defaults = default_colors();
        assert_eq!(colors.primary, defaults.primary, "blank primary -> default");
        assert_eq!(colors.secondary, defaults.secondary, "blank secondary -> default");
        assert_eq!(colors.background, Color::Reset, "blank background -> reset");
        assert_eq!(colors.active_border, None, "blank active-border -> name hue");
        assert_eq!(colors.inactive_border, None, "blank inactive-border -> map");
        assert_eq!(
            colors.borders.get("modules"),
            Some(&Color::Cyan),
            "an empty borders map keeps the default entry"
        );

        // Set scalars and map entries: the custom colours.
        std::fs::write(
            &path,
            r##"{"colorscheme":{"primary":"#112233","secondary":"yellow","background":"#000000","active-border":"red","inactive-border":"blue","borders":{"modules":"green"}}}"##,
        )
        .unwrap();
        let colors = load_colors(&path);
        assert_eq!(colors.primary, Color::Rgb(0x11, 0x22, 0x33));
        assert_eq!(colors.secondary, Color::Yellow);
        assert_eq!(colors.background, Color::Rgb(0, 0, 0));
        assert_eq!(colors.active_border, Some(Color::Red));
        assert_eq!(colors.inactive_border, Some(Color::Blue));
        assert_eq!(colors.borders.get("modules"), Some(&Color::Green));
        assert_eq!(
            colors.status_color("building"),
            Color::Rgb(119, 158, 203),
            "a status missing from the colorscheme still gets its default"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_color_config_is_honored_and_colorscheme_wins() {
        let (dir, path) = temp_config_path("config.json");
        // The legacy file beside the TUI config.json.
        std::fs::write(
            dir.join("color_config.json"),
            r#"{"status":{"connected":"red"},"borders":{"modules":"yellow"}}"#,
        )
        .unwrap();
        // The new colorscheme overrides `modules` but says nothing about
        // `connected`, so the legacy value for `connected` must survive.
        std::fs::write(
            &path,
            r#"{"colorscheme":{"borders":{"modules":"green"}}}"#,
        )
        .unwrap();
        let colors = load_colors(&path);
        assert_eq!(
            colors.status_color("connected"),
            Color::Red,
            "the legacy color_config.json is still read"
        );
        assert_eq!(
            colors.borders.get("modules"),
            Some(&Color::Green),
            "the colorscheme wins over the legacy file"
        );

        // A missing file is pure defaults.
        let colors = load_colors(&dir.join("nope.json"));
        assert_eq!(colors.status_color("building"), Color::Rgb(119, 158, 203));
        // ...but a sibling legacy file is still picked up.
        std::fs::write(dir.join("color_config.json"), r#"{"status":{"building":"blue"}}"#).unwrap();
        let colors = load_colors(&dir.join("nope.json"));
        assert_eq!(colors.status_color("building"), Color::Blue);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

use std::collections::HashMap;
use std::path::PathBuf;

use ratatui::style::Color;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ColorFile {
    #[serde(default)]
    borders: HashMap<String, String>,
    #[serde(default)]
    status: HashMap<String, String>,
    #[serde(default)]
    platforms: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ColorConfig {
    pub borders: HashMap<String, Color>,
    pub status: HashMap<String, Color>,
    pub platforms: HashMap<String, Color>,
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

/// Load the color config, OVERLAYING the file's entries on top of the built-in
/// defaults. Merging (rather than replacing) means a config that omits a key —
/// or predates a newly-added status like `building` — still gets a sensible
/// default instead of falling back to white.
pub fn load_colors(path: &PathBuf) -> ColorConfig {
    let mut colors = default_colors();
    let Ok(content) = std::fs::read_to_string(path) else {
        return colors;
    };
    let Ok(file) = serde_json::from_str::<ColorFile>(&content) else {
        return colors;
    };
    for (k, v) in &file.borders {
        colors.borders.insert(k.clone(), parse_color(v));
    }
    for (k, v) in &file.status {
        colors.status.insert(k.clone(), parse_color(v));
    }
    for (k, v) in &file.platforms {
        colors.platforms.insert(k.clone(), parse_color(v));
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

    ColorConfig { borders, status, platforms }
}

impl ColorConfig {
    pub fn border_color(&self, window_name: &str) -> Color {
        if let Some(c) = self.borders.get(window_name) {
            *c
        } else {
            Color::DarkGray
        }
    }

    pub fn active_border_color(&self, window_name: &str) -> Color {
        if let Some(c) = self.borders.get(window_name) {
            *c
        } else {
            Color::Gray
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
    fn file_colors_merge_over_defaults() {
        let dir = std::env::temp_dir().join(format!("ckt-colors-{}", uuid::Uuid::now_v7()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("color_config.json");
        // A file that overrides `connected` but OMITS `building`.
        std::fs::write(&path, r#"{"status":{"connected":"red"}}"#).unwrap();
        let colors = load_colors(&path);
        assert_eq!(colors.status_color("connected"), Color::Red, "file overrides the default");
        assert_eq!(
            colors.status_color("building"),
            Color::Rgb(119, 158, 203),
            "a key missing from the file falls back to the default, not white"
        );
        // A missing file is pure defaults.
        let colors = load_colors(&dir.join("nope.json"));
        assert_eq!(colors.status_color("building"), Color::Rgb(119, 158, 203));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

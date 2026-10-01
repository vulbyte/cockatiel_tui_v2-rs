//! Text-to-speech backend for accessibility.
//!
//! Cockatiel's TUI is a structured layout (a BSP tree of panes), not a flat
//! character stream — a screen reader parsing the raw terminal would garble
//! side-by-side panes into jargon. Instead, the app captures STRUCTURAL
//! context and funnels semantic descriptions to the OS TTS engine.
//!
//! The backend is pluggable so the same announcement code works on every OS:
//!   * macOS   — the `say` binary (NSSpeechSynthesizer under the hood)
//!   * Linux   — speech-dispatcher's `spd-say`
//!   * Windows — PowerShell SAPI (System.Speech)
//!
//! Announcements are fire-and-forget: a slow TTS must never block the render
//! or input loop, so each speak runs in a detached thread.

use std::process::Command;

/// A TTS backend. Implementations must not block the caller for long.
pub trait TtsBackend: Send + Sync {
    fn speak(&self, text: &str);
}

/// The default backend for the current OS.
pub fn default_backend() -> Box<dyn TtsBackend> {
    #[cfg(target_os = "macos")]
    { Box::new(SayBackend) }
    #[cfg(target_os = "linux")]
    { Box::new(SpdSayBackend) }
    #[cfg(target_os = "windows")]
    { Box::new(SapiBackend) }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    { Box::new(NoopBackend) }
}

/// macOS: `say` is always present.
#[cfg(target_os = "macos")]
pub struct SayBackend;
#[cfg(target_os = "macos")]
impl TtsBackend for SayBackend {
    fn speak(&self, text: &str) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = Command::new("say").arg(text).status();
        });
    }
}

/// Linux: speech-dispatcher's `spd-say`. Falls back silently if absent.
#[cfg(target_os = "linux")]
pub struct SpdSayBackend;
#[cfg(target_os = "linux")]
impl TtsBackend for SpdSayBackend {
    fn speak(&self, text: &str) {
        let text = text.to_string();
        std::thread::spawn(move || {
            let _ = Command::new("spd-say").arg("-w").arg(text).status();
        });
    }
}

/// Windows: PowerShell + System.Speech (SAPI).
#[cfg(target_os = "windows")]
pub struct SapiBackend;
#[cfg(target_os = "windows")]
impl TtsBackend for SapiBackend {
    fn speak(&self, text: &str) {
        let text = text.replace('"', "'");
        let script = format!("Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak('{}')", text);
        std::thread::spawn(move || {
            let _ = Command::new("powershell")
                .args(["-NoProfile", "-Command", &script])
                .status();
        });
    }
}

/// Non-supported OS: silently drop.
#[allow(dead_code)]
pub struct NoopBackend;
impl TtsBackend for NoopBackend {
    fn speak(&self, _text: &str) {}
}

/// A no-op backend used when accessibility is disabled.
pub struct MutedBackend;
impl TtsBackend for MutedBackend {
    fn speak(&self, _text: &str) {}
}
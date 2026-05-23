//! homn-face — Phase 2 spike.
//!
//! Opens a small transparent always-on-top frameless window that renders the idle
//! face `◕ ◡ ◕`. No event-bus wiring yet; the spike's sole job is to validate that
//! Tauri 2 + webkit2gtk-4.1 + KWin Wayland produce a usable window on this host.
//! See `docs/superpowers/specs/2026-05-23-phase-2-face-spike-design.md`.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running the homn-face Tauri application");
}

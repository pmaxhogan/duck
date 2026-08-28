//! The ducking engine: Voicemeeter geometry, config, the state machine, and the
//! FFI wrapper. Independent of Tauri so it can be unit-tested on its own.

pub mod config;
pub mod ducker;
pub mod layout;
pub mod voicemeeter;

pub use config::{Config, Rule, Source};
pub use ducker::{Ducker, State, StripControl};
pub use voicemeeter::{color_strips_for, read_levels, VoicemeeterApi};

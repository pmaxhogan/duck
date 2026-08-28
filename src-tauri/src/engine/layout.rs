//! Voicemeeter channel/strip geometry and small numeric helpers.
//!
//! Pure functions only - no FFI, no I/O - so this compiles and tests anywhere.

pub const DLL_NAME_64: &str = "VoicemeeterRemote64.dll";

pub const GAIN_MIN: f32 = -60.0;
pub const GAIN_MAX: f32 = 12.0;
pub const SILENCE_DB: f32 = -120.0;

/// `FadeTo` lands noticeably later than the requested time; the settle window
/// before we look for manual fader moves is scaled by this.
pub const FADE_SLACK_MULT: f64 = 1.5;
pub const FADE_SLACK_EXTRA_MS: f64 = 100.0;

/// Voicemeeter kind -> (hardware input strips, virtual input strips).
/// In the level-meter numbering hardware strips are stereo, virtual strips 8-wide.
pub fn strip_layout(kind: i32) -> Option<(u32, u32)> {
    match kind {
        1 => Some((2, 1)),
        2 => Some((3, 2)),
        3 => Some((5, 3)),
        _ => None,
    }
}

pub fn kind_name(kind: i32) -> &'static str {
    match kind {
        1 => "Voicemeeter",
        2 => "Voicemeeter Banana",
        3 => "Voicemeeter Potato",
        _ => "Voicemeeter (unknown)",
    }
}

/// Level-meter channel indices (pre-fader bus 0) for a strip.
pub fn strip_channels(kind: i32, strip: u32) -> std::ops::Range<i32> {
    let (hardware, _) = strip_layout(kind).expect("valid voicemeeter kind");
    if strip < hardware {
        let start = (strip * 2) as i32;
        start..start + 2
    } else {
        let first = (hardware * 2 + (strip - hardware) * 8) as i32;
        first..first + 8
    }
}

/// Only hardware input strips expose the Pan/Color pads; virtual strips get an EQ.
pub fn has_color_pad(kind: i32, strip: u32) -> bool {
    strip < strip_layout(kind).expect("valid voicemeeter kind").0
}

pub fn strip_count(kind: i32) -> u32 {
    let (hw, virt) = strip_layout(kind).expect("valid voicemeeter kind");
    hw + virt
}

pub fn to_db(level: f32) -> f32 {
    if level > 1e-6 {
        20.0 * level.log10()
    } else {
        SILENCE_DB
    }
}

/// Clamp to the fader range and round to two decimals (matches Voicemeeter's display).
pub fn clamp_gain(gain: f32) -> f32 {
    let clamped = gain.clamp(GAIN_MIN, GAIN_MAX);
    (clamped * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn banana_channels() {
        assert_eq!(strip_channels(2, 0), 0..2);
        assert_eq!(strip_channels(2, 2), 4..6);
        assert_eq!(strip_channels(2, 3), 6..14);
        assert_eq!(strip_channels(2, 4), 14..22);
    }

    #[test]
    fn color_pad_only_on_hardware_strips() {
        assert!(has_color_pad(2, 2));
        assert!(!has_color_pad(2, 3));
    }

    #[test]
    fn db_conversion() {
        assert_eq!(to_db(0.0), SILENCE_DB);
        assert!((to_db(1.0) - 0.0).abs() < 1e-4);
        assert!((to_db(0.1) - -20.0).abs() < 1e-3);
    }

    #[test]
    fn strip_counts() {
        assert_eq!(strip_count(2), 5);
        assert_eq!(strip_count(3), 8);
    }
}

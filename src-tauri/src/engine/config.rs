//! Configuration model: parse, validate, and serialise ducking rules.
//!
//! Reading is TOML (via the `toml` crate + serde). Writing uses a small hand
//! dumper so the on-disk file stays tidy and comment-friendly regardless of how
//! the `toml` serializer would lay out nested arrays-of-tables. The structs also
//! derive `Serialize` so the whole config round-trips to the UI as JSON.

use std::fmt;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::layout::{kind_name, strip_count};

pub const DEFAULT_THRESHOLD_DB: f32 = -40.0;
pub const DEFAULT_HOLD_MS: f64 = 1000.0;
pub const DEFAULT_DUCK_DB: f32 = -8.0;
pub const DEFAULT_ATTACK_MS: f64 = 60.0;
pub const DEFAULT_RELEASE_MS: f64 = 350.0;
pub const DEFAULT_POLL_MS: f64 = 50.0;

fn default_threshold() -> f32 {
    DEFAULT_THRESHOLD_DB
}
fn default_hold() -> f64 {
    DEFAULT_HOLD_MS
}
fn default_duck() -> f32 {
    DEFAULT_DUCK_DB
}
fn default_attack() -> f64 {
    DEFAULT_ATTACK_MS
}
fn default_release() -> f64 {
    DEFAULT_RELEASE_MS
}
fn default_poll() -> f64 {
    DEFAULT_POLL_MS
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub strip: u32,
    #[serde(default = "default_threshold")]
    pub threshold_db: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ColorCfg {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub sources: Vec<Source>,
    pub targets: Vec<u32>,
    #[serde(default = "default_duck")]
    pub duck_db: f32,
    #[serde(default = "default_attack")]
    pub attack_ms: f64,
    #[serde(default = "default_release")]
    pub release_ms: f64,
    #[serde(default = "default_hold")]
    pub hold_ms: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorCfg>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "default_poll")]
    pub poll_ms: f64,
    #[serde(rename = "rule", default)]
    pub rules: Vec<Rule>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ConfigError {}

impl Rule {
    pub fn color_tuple(&self) -> Option<(f32, f32)> {
        self.color.map(|c| (c.x, c.y))
    }
}

// --------------------------------------------------------------------------- parse

pub fn parse_config(text: &str) -> Result<Config, ConfigError> {
    let cfg: Config = toml::from_str(text).map_err(|e| ConfigError(format!("invalid TOML: {e}")))?;
    validate_structure(&cfg)?;
    Ok(cfg)
}

/// Checks that don't need to know which Voicemeeter is running.
pub fn validate_structure(cfg: &Config) -> Result<(), ConfigError> {
    if cfg.rules.is_empty() {
        return Err(ConfigError("no [[rule]] defined".into()));
    }
    let mut owner: std::collections::HashMap<u32, usize> = std::collections::HashMap::new();
    for (i, rule) in cfg.rules.iter().enumerate() {
        if rule.sources.is_empty() || rule.targets.is_empty() {
            return Err(ConfigError(format!(
                "rule {i}: needs at least one source and one target"
            )));
        }
        let src: std::collections::HashSet<u32> = rule.sources.iter().map(|s| s.strip).collect();
        for t in &rule.targets {
            if src.contains(t) {
                return Err(ConfigError(format!(
                    "rule {i}: strip {t} is both a source and a target"
                )));
            }
            if let Some(prev) = owner.insert(*t, i) {
                return Err(ConfigError(format!(
                    "strip {t} is a target of both rule {prev} and rule {i}"
                )));
            }
        }
        if let Some(c) = rule.color {
            if !(-0.5..=0.5).contains(&c.x) || !(0.0..=1.0).contains(&c.y) {
                return Err(ConfigError(format!(
                    "rule {i}: color.x must be -0.5..0.5 and color.y 0..1"
                )));
            }
        }
    }
    Ok(())
}

/// Checks that need the running Voicemeeter's strip count.
pub fn validate_strips(cfg: &Config, kind: i32) -> Result<(), ConfigError> {
    let n = strip_count(kind);
    for rule in &cfg.rules {
        let strips = rule
            .sources
            .iter()
            .map(|s| s.strip)
            .chain(rule.targets.iter().copied());
        for s in strips {
            if s >= n {
                return Err(ConfigError(format!(
                    "strip {s} does not exist; {} has strips 0..{}",
                    kind_name(kind),
                    n - 1
                )));
            }
        }
    }
    Ok(())
}

// --------------------------------------------------------------------------- serialise

fn fmt_num(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        // trim to a sensible precision without trailing zeros
        let s = format!("{:.3}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

pub fn dump_config(cfg: &Config) -> String {
    let mut out = String::new();
    out.push_str("# Duck configuration - edited by the settings window, or by hand.\n");
    out.push_str("# Strip numbers are 0-based, left to right in the Voicemeeter window.\n\n");
    out.push_str(&format!("poll_ms = {}\n", fmt_num(cfg.poll_ms)));
    for rule in &cfg.rules {
        out.push_str("\n[[rule]]\nsources = [\n");
        for s in &rule.sources {
            out.push_str(&format!(
                "  {{ strip = {}, threshold_db = {} }},\n",
                s.strip,
                fmt_num(s.threshold_db as f64)
            ));
        }
        out.push_str("]\n");
        let targets: Vec<String> = rule.targets.iter().map(|t| t.to_string()).collect();
        out.push_str(&format!("targets    = [{}]\n", targets.join(", ")));
        out.push_str(&format!("duck_db    = {}\n", fmt_num(rule.duck_db as f64)));
        out.push_str(&format!("attack_ms  = {}\n", fmt_num(rule.attack_ms)));
        out.push_str(&format!("release_ms = {}\n", fmt_num(rule.release_ms)));
        out.push_str(&format!("hold_ms    = {}\n", fmt_num(rule.hold_ms)));
        if let Some(c) = rule.color {
            out.push_str(&format!(
                "color      = {{ x = {}, y = {} }}\n",
                fmt_num(c.x as f64),
                fmt_num(c.y as f64)
            ));
        }
    }
    out
}

// --------------------------------------------------------------------------- files

pub fn load_config(path: &Path) -> Result<Config, ConfigError> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| ConfigError(format!("cannot read {}: {e}", path.display())))?;
    parse_config(&text)
}

pub fn save_config(path: &Path, cfg: &Config) -> Result<(), ConfigError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ConfigError(format!("cannot create {}: {e}", parent.display())))?;
    }
    std::fs::write(path, dump_config(cfg))
        .map_err(|e| ConfigError(format!("cannot write {}: {e}", path.display())))
}

/// A safe starter rule for first run: duck strip 1 when strip 0 is active.
pub fn default_config() -> Config {
    Config {
        poll_ms: DEFAULT_POLL_MS,
        rules: vec![Rule {
            sources: vec![Source {
                strip: 0,
                threshold_db: DEFAULT_THRESHOLD_DB,
            }],
            targets: vec![1],
            duck_db: DEFAULT_DUCK_DB,
            attack_ms: DEFAULT_ATTACK_MS,
            release_ms: DEFAULT_RELEASE_MS,
            hold_ms: DEFAULT_HOLD_MS,
            color: None,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_rule_with_per_source_threshold() {
        let cfg = parse_config(
            r#"
            poll_ms = 25
            [[rule]]
            sources = [ { strip = 0 }, { strip = 3, threshold_db = -35 } ]
            targets = [1, 2]
            duck_db = -12
            attack_ms = 100
            release_ms = 500
            hold_ms = 800
            color = { x = -0.5, y = 0.2 }
        "#,
        )
        .unwrap();
        assert_eq!(cfg.poll_ms, 25.0);
        let r = &cfg.rules[0];
        assert_eq!(r.sources[0], Source { strip: 0, threshold_db: -40.0 });
        assert_eq!(r.sources[1], Source { strip: 3, threshold_db: -35.0 });
        assert_eq!(r.targets, vec![1, 2]);
        assert_eq!(r.duck_db, -12.0);
        assert_eq!(r.color, Some(ColorCfg { x: -0.5, y: 0.2 }));
    }

    #[test]
    fn applies_defaults() {
        let cfg = parse_config("[[rule]]\nsources=[{strip=0}]\ntargets=[1]\n").unwrap();
        let r = &cfg.rules[0];
        assert_eq!(r.sources[0].threshold_db, -40.0);
        assert_eq!((r.hold_ms, r.duck_db, r.attack_ms, r.release_ms), (1000.0, -8.0, 60.0, 350.0));
        assert!(r.color.is_none());
        assert_eq!(cfg.poll_ms, 50.0);
    }

    #[test]
    fn rejects_overlapping_targets() {
        let e = parse_config(
            "[[rule]]\nsources=[{strip=0}]\ntargets=[1]\n[[rule]]\nsources=[{strip=3}]\ntargets=[1]\n",
        );
        assert!(e.is_err());
    }

    #[test]
    fn rejects_source_that_is_also_target() {
        assert!(parse_config("[[rule]]\nsources=[{strip=0},{strip=1}]\ntargets=[1]\n").is_err());
    }

    #[test]
    fn rejects_empty() {
        assert!(parse_config("poll_ms = 50\n").is_err());
        assert!(parse_config("[[rule]]\nsources=[]\ntargets=[1]\n").is_err());
    }

    #[test]
    fn rejects_bad_color() {
        assert!(parse_config("[[rule]]\nsources=[{strip=0}]\ntargets=[1]\ncolor={x=2,y=0}\n").is_err());
    }

    #[test]
    fn validates_strips_against_kind() {
        let cfg = parse_config("[[rule]]\nsources=[{strip=0}]\ntargets=[7]\n").unwrap();
        assert!(validate_strips(&cfg, 2).is_err()); // banana has 0..4
        assert!(validate_strips(&cfg, 3).is_ok()); // potato has 0..7
    }

    #[test]
    fn dump_round_trips() {
        let cfg = parse_config(
            "[[rule]]\nsources=[{strip=0,threshold_db=-40},{strip=3,threshold_db=-35}]\ntargets=[1,2]\ncolor={x=-0.2,y=0}\n",
        )
        .unwrap();
        let dumped = dump_config(&cfg);
        let reparsed = parse_config(&dumped).unwrap();
        assert_eq!(cfg, reparsed);
    }

    #[test]
    fn default_config_is_valid() {
        validate_structure(&default_config()).unwrap();
    }
}

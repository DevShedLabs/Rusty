use std::path::PathBuf;
use anyhow::Result;
use serde::{Deserialize, Serialize};

// ── Color ─────────────────────────────────────────────────────────────────────

/// An RGB color expressed as a hex string in config (`"#131313"`)
/// and stored as [r, g, b] bytes internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color(pub [u8; 3]);

impl Color {
    pub fn to_rgba(self) -> [u8; 4] {
        [self.0[0], self.0[1], self.0[2], 0xff]
    }
    pub fn r(self) -> u8 { self.0[0] }
    pub fn g(self) -> u8 { self.0[1] }
    pub fn b(self) -> u8 { self.0[2] }
}

impl Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format!("#{:02x}{:02x}{:02x}", self.0[0], self.0[1], self.0[2]))
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        parse_hex(&s).map_err(serde::de::Error::custom)
    }
}

fn parse_hex(s: &str) -> Result<Color> {
    let s = s.trim().trim_start_matches('#');
    anyhow::ensure!(s.len() == 6, "color must be 6 hex digits, got {:?}", s);
    let r = u8::from_str_radix(&s[0..2], 16)?;
    let g = u8::from_str_radix(&s[2..4], 16)?;
    let b = u8::from_str_radix(&s[4..6], 16)?;
    Ok(Color([r, g, b]))
}

// ── Palette ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Palette {
    // Terminal background / foreground
    pub background: Color,
    pub foreground: Color,

    // Cursor and selection
    pub cursor:           Color,
    pub selection_bg:     Color,
    pub selection_fg:     Color,

    // Normal ANSI colors 0-7
    pub black:   Color,
    pub red:     Color,
    pub green:   Color,
    pub yellow:  Color,
    pub blue:    Color,
    pub magenta: Color,
    pub cyan:    Color,
    pub white:   Color,

    // Bright ANSI colors 8-15
    pub bright_black:   Color,
    pub bright_red:     Color,
    pub bright_green:   Color,
    pub bright_yellow:  Color,
    pub bright_blue:    Color,
    pub bright_magenta: Color,
    pub bright_cyan:    Color,
    pub bright_white:   Color,
}

impl Palette {
    /// Convert to the flat 16-entry ANSI array expected by the renderer.
    pub fn to_ansi16(&self) -> [[u8; 4]; 16] {
        [
            self.black.to_rgba(),
            self.red.to_rgba(),
            self.green.to_rgba(),
            self.yellow.to_rgba(),
            self.blue.to_rgba(),
            self.magenta.to_rgba(),
            self.cyan.to_rgba(),
            self.white.to_rgba(),
            self.bright_black.to_rgba(),
            self.bright_red.to_rgba(),
            self.bright_green.to_rgba(),
            self.bright_yellow.to_rgba(),
            self.bright_blue.to_rgba(),
            self.bright_magenta.to_rgba(),
            self.bright_cyan.to_rgba(),
            self.bright_white.to_rgba(),
        ]
    }
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            background:     Color([0x13, 0x13, 0x13]),
            foreground:     Color([0xd8, 0xd8, 0xd8]),
            cursor:         Color([0xf8, 0xf8, 0xf0]),
            selection_bg:   Color([0x26, 0x4f, 0x78]),
            selection_fg:   Color([0xff, 0xff, 0xff]),

            black:          Color([0x13, 0x13, 0x13]),
            red:            Color([0xe0, 0x5a, 0x4f]),
            green:          Color([0x87, 0xc3, 0x6c]),
            yellow:         Color([0xe5, 0xc0, 0x76]),
            blue:           Color([0x6b, 0xa3, 0xe0]),
            magenta:        Color([0xc0, 0x7d, 0xd4]),
            cyan:           Color([0x5b, 0xc8, 0xd4]),
            white:          Color([0xc5, 0xc5, 0xc5]),

            bright_black:   Color([0x52, 0x52, 0x52]),
            bright_red:     Color([0xff, 0x7a, 0x70]),
            bright_green:   Color([0xa8, 0xe0, 0x8a]),
            bright_yellow:  Color([0xff, 0xdc, 0x9a]),
            bright_blue:    Color([0x8f, 0xc3, 0xff]),
            bright_magenta: Color([0xda, 0x9f, 0xf5]),
            bright_cyan:    Color([0x7f, 0xe3, 0xee]),
            bright_white:   Color([0xff, 0xff, 0xff]),
        }
    }
}

// ── Font ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FontConfig {
    pub size:   f32,
    /// Multiplier for natural scroll direction. 1.0 = natural, -1.0 = reversed.
    pub family: String,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self { size: 16.0, family: "JetBrains Mono".into() }
    }
}

// ── Scrolling ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ScrollConfig {
    pub natural:    bool,
    pub lines:      usize,
    pub history:    usize,
}

impl Default for ScrollConfig {
    fn default() -> Self {
        Self { natural: true, lines: 3, history: 10_000 }
    }
}

// ── Window ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowConfig {
    /// Background opacity: 0.0 = fully transparent, 1.0 = fully opaque.
    pub opacity: f32,
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self { opacity: 1.0 }
    }
}

// ── Tabs ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TabsConfig {
    /// Background of the tab bar.
    pub bar_bg:        Color,
    /// Foreground (text) of inactive tabs.
    pub bar_fg:        Color,
    /// Background of the active tab.
    pub active_bg:     Color,
    /// Foreground (text) of the active tab.
    pub active_fg:     Color,
    /// Accent line color shown on the active pane when splits are open.
    pub active_border: Color,
    /// Divider color between tabs and between split panes.
    pub separator:     Color,
}

impl Default for TabsConfig {
    fn default() -> Self {
        Self {
            bar_bg:        Color([0x1e, 0x1e, 0x2e]),
            bar_fg:        Color([0x88, 0x88, 0xaa]),
            active_bg:     Color([0x31, 0x31, 0x4a]),
            active_fg:     Color([0xe0, 0xe0, 0xff]),
            active_border: Color([0x89, 0xb4, 0xfa]),
            separator:     Color([0x44, 0x44, 0x66]),
        }
    }
}

// ── Hints ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HintsConfig {
    /// Show ghost (inline) history suggestions and history entries in the
    /// completion popup. Off by default — set to true to enable.
    pub fuzzy_history: bool,
}

impl Default for HintsConfig {
    fn default() -> Self {
        Self { fuzzy_history: false }
    }
}

// ── Top-level config ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub palette:  Palette,
    pub font:     FontConfig,
    pub scroll:   ScrollConfig,
    pub window:   WindowConfig,
    pub tabs:     TabsConfig,
    pub hints:    HintsConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            palette: Palette::default(),
            font:    FontConfig::default(),
            scroll:  ScrollConfig::default(),
            window:  WindowConfig::default(),
            tabs:    TabsConfig::default(),
            hints:   HintsConfig::default(),
        }
    }
}

impl Config {
    /// Load from `~/.config/rusty/config.toml`.
    /// If the file doesn't exist, write the default and return it.
    pub fn load() -> Self {
        match Self::try_load() {
            Ok(cfg) => cfg,
            Err(e)  => {
                tracing::warn!("config error: {e} — using defaults");
                Self::default()
            }
        }
    }

    fn try_load() -> Result<Self> {
        let path = config_path();
        if !path.exists() {
            let cfg = Self::default();
            cfg.write_default(&path)?;
            return Ok(cfg);
        }
        let text = std::fs::read_to_string(&path)?;
        let cfg: Self = toml::from_str(&text)?;
        tracing::info!("loaded config from {}", path.display());
        Ok(cfg)
    }

    fn write_default(&self, path: &PathBuf) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, format!("{HEADER}{text}"))?;
        tracing::info!("wrote default config to {}", path.display());
        Ok(())
    }
}

fn config_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config").join("rusty").join("config.toml")
}

const HEADER: &str = "\
# rusty terminal configuration
# All colors are hex strings: \"#rrggbb\"
# Edit this file and restart rusty to apply changes.
#
# [window]
# opacity = 1.0   # 0.0 (transparent) → 1.0 (opaque)

";

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScalingMode {
    Linear,
    Decibel,
}

impl ScalingMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ScalingMode::Linear => "linear",
            ScalingMode::Decibel => "decibel",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMethod {
    Pulse,
    Pipewire,
    Alsa,
    Fifo,
    Cpal,
    Test,
}

impl InputMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            InputMethod::Pulse => "pulse",
            InputMethod::Pipewire => "pipewire",
            InputMethod::Alsa => "alsa",
            InputMethod::Fifo => "fifo",
            InputMethod::Cpal => "cpal",
            InputMethod::Test => "test",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputMethod {
    Noncurses,
    Ncurses,
    Raw,
}

impl OutputMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutputMethod::Noncurses => "noncurses",
            OutputMethod::Ncurses => "ncurses",
            OutputMethod::Raw => "raw",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Bottom = 0,
    Top = 1,
    Left = 2,
    Right = 3,
    Horizontal = 4,
}

impl Orientation {
    pub fn as_str(&self) -> &'static str {
        match self {
            Orientation::Bottom => "bottom",
            Orientation::Top => "top",
            Orientation::Left => "left",
            Orientation::Right => "right",
            Orientation::Horizontal => "horizontal",
        }
    }

    pub fn cycle(&self, output_method: OutputMethod) -> Self {
        match output_method {
            OutputMethod::Noncurses => match self {
                Orientation::Bottom => Orientation::Top,
                Orientation::Top => Orientation::Left,
                Orientation::Left => Orientation::Right,
                Orientation::Right => Orientation::Horizontal,
                Orientation::Horizontal => Orientation::Bottom,
            },
            OutputMethod::Ncurses => match self {
                Orientation::Bottom => Orientation::Top,
                Orientation::Top => Orientation::Left,
                Orientation::Left => Orientation::Right,
                Orientation::Right => Orientation::Bottom,
                Orientation::Horizontal => Orientation::Bottom,
            },
            OutputMethod::Raw => *self,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonoOption {
    Average,
    Left,
    Right,
}

impl MonoOption {
    pub fn as_str(&self) -> &'static str {
        match self {
            MonoOption::Average => "average",
            MonoOption::Left => "left",
            MonoOption::Right => "right",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataFormat {
    Binary,
    Ascii,
}

impl DataFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            DataFormat::Binary => "binary",
            DataFormat::Ascii => "ascii",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BitFormat {
    Bits8,
    Bits16,
}

impl BitFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            BitFormat::Bits8 => "8bit",
            BitFormat::Bits16 => "16bit",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum XAxis {
    None,
    Frequency,
}

impl XAxis {
    pub fn as_str(&self) -> &'static str {
        match self {
            XAxis::None => "none",
            XAxis::Frequency => "scale",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendDirection {
    Up,
    Down,
    Left,
    Right,
}

impl BlendDirection {
    pub fn as_str(&self) -> &'static str {
        match self {
            BlendDirection::Up => "up",
            BlendDirection::Down => "down",
            BlendDirection::Left => "left",
            BlendDirection::Right => "right",
        }
    }
}

/// How the oscilloscope draws the trace: a thin line (half-block glyphs) or a
/// filled area (8-level sub-character blocks, much smoother).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaveformStyle {
    Line,
    Area,
}

impl WaveformStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            WaveformStyle::Line => "line",
            WaveformStyle::Area => "area",
        }
    }
}

#[derive(Debug, Clone)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalColor {
    Default,
    Black,
    Red,
    Green,
    Yellow,
    Blue,
    Magenta,
    Cyan,
    White,
    Rgb(u8, u8, u8),
}

impl TerminalColor {
    pub fn parse(s: &str) -> Self {
        let clean = s.trim().trim_matches('\x27').trim_matches('"').trim();
        if clean.starts_with('#') {
            let hex = &clean[1..];
            if hex.len() == 6 {
                if let (Ok(r), Ok(g), Ok(b)) = (
                    u8::from_str_radix(&hex[0..2], 16),
                    u8::from_str_radix(&hex[2..4], 16),
                    u8::from_str_radix(&hex[4..6], 16),
                ) {
                    return TerminalColor::Rgb(r, g, b);
                }
            }
        }
        match clean.to_lowercase().as_str() {
            "black" => TerminalColor::Black,
            "red" => TerminalColor::Red,
            "green" => TerminalColor::Green,
            "yellow" => TerminalColor::Yellow,
            "blue" => TerminalColor::Blue,
            "magenta" => TerminalColor::Magenta,
            "cyan" => TerminalColor::Cyan,
            "white" => TerminalColor::White,
            _ => TerminalColor::Default,
        }
    }

    /// INI representation: named colors as lowercase names, RGB as #rrggbb.
    pub fn to_ini(&self) -> String {
        match self {
            TerminalColor::Default => "default".to_string(),
            TerminalColor::Black => "black".to_string(),
            TerminalColor::Red => "red".to_string(),
            TerminalColor::Green => "green".to_string(),
            TerminalColor::Yellow => "yellow".to_string(),
            TerminalColor::Blue => "blue".to_string(),
            TerminalColor::Magenta => "magenta".to_string(),
            TerminalColor::Cyan => "cyan".to_string(),
            TerminalColor::White => "white".to_string(),
            TerminalColor::Rgb(r, g, b) => format!("#{:02x}{:02x}{:02x}", r, g, b),
        }
    }

    pub fn to_rgb(&self) -> Option<RgbColor> {
        match self {
            TerminalColor::Rgb(r, g, b) => Some(RgbColor {
                r: *r,
                g: *g,
                b: *b,
            }),
            TerminalColor::Black => Some(RgbColor { r: 0, g: 0, b: 0 }),
            TerminalColor::Red => Some(RgbColor { r: 255, g: 0, b: 0 }),
            TerminalColor::Green => Some(RgbColor { r: 0, g: 255, b: 0 }),
            TerminalColor::Yellow => Some(RgbColor {
                r: 255,
                g: 255,
                b: 0,
            }),
            TerminalColor::Blue => Some(RgbColor { r: 0, g: 0, b: 255 }),
            TerminalColor::Magenta => Some(RgbColor {
                r: 255,
                g: 0,
                b: 255,
            }),
            TerminalColor::Cyan => Some(RgbColor {
                r: 0,
                g: 255,
                b: 255,
            }),
            TerminalColor::White => Some(RgbColor {
                r: 255,
                g: 255,
                b: 255,
            }),
            TerminalColor::Default => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneralConfig {
    pub framerate: u32,
    pub autosens: u32,
    pub sensitivity: f64,
    pub scaling: ScalingMode,
    pub bars: usize,
    pub bar_width: usize,
    pub bar_spacing: usize,
    pub center_align: bool,
    /// Left/Right at the end of their range jump to the opposite end instead of
    /// stopping. Off by default: bounded behaviour matches CAVA and keeps the
    /// keypress predictable (the status overlay reports the clamped value).
    pub wrap_bars: bool,
    pub max_height: f64,
    pub lower_cutoff_freq: u32,
    pub higher_cutoff_freq: u32,
    pub sleep_timer: u32,
    pub live_config: bool,
}

#[derive(Debug, Clone)]
pub struct InputConfig {
    pub method: InputMethod,
    pub source: String,
    pub sample_rate: u32,
    pub sample_bits: u32,
    pub channels: u32,
}

#[derive(Debug, Clone)]
pub struct OutputConfig {
    pub method: OutputMethod,
    pub orientation: Orientation,
    pub channels: u32,
    pub mono_option: MonoOption,
    pub reverse: bool,
    pub split_stereo: bool,
    pub raw_target: String,
    pub data_format: DataFormat,
    pub bit_format: BitFormat,
    pub ascii_max_range: u32,
    pub bar_delimiter: u8,
    pub frame_delimiter: u8,
    pub xaxis: XAxis,
    pub show_idle_bar_heads: bool,
    pub waveform: bool,
    pub waveform_style: WaveformStyle,
    pub waveform_baseline: bool,
    pub waveform_dynamics: bool,
    pub waveform_graticule: bool,
    pub bar_reflection: bool,
    pub stereo_divider: bool,
    pub baseline_ruler: bool,
    pub bar_cap: bool,
    pub bright_peaks: bool,
    pub beat_pulse: bool,
    pub ascii_glyphs: bool,
    pub reduce_motion: bool,
    pub draw_peaks: bool,
    pub peak_decay: f64,
}

#[derive(Debug, Clone)]
pub struct ColorConfig {
    pub background: TerminalColor,
    pub foreground: TerminalColor,
    pub gradient: bool,
    pub gradient_colors: Vec<TerminalColor>,
    pub horizontal_gradient: bool,
    pub horizontal_gradient_colors: Vec<TerminalColor>,
    pub blend_direction: BlendDirection,
    /// Hue sweeps across the bar axis (low = warm, high = cool), overriding
    /// the palette gradients.
    pub spectrum: bool,
    /// A dim vertical ramp behind the bars instead of a flat background.
    pub background_gradient: bool,
    pub theme: String,
}

#[derive(Debug, Clone)]
pub struct SmoothingConfig {
    pub monstercat: f64,
    pub waves: bool,
    pub noise_reduction: f64,
    pub gravity: f64,
}

#[derive(Debug, Clone)]
pub struct Config {
    pub general: GeneralConfig,
    pub input: InputConfig,
    pub output: OutputConfig,
    pub color: ColorConfig,
    pub smoothing: SmoothingConfig,
    pub eq: BTreeMap<usize, f64>,
    pub config_path: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: GeneralConfig {
                framerate: 60,
                autosens: 1,
                sensitivity: 100.0,
                scaling: ScalingMode::Linear,
                bars: 0,
                bar_width: 2,
                bar_spacing: 1,
                center_align: true,
                wrap_bars: false,
                max_height: 1.0,
                lower_cutoff_freq: 50,
                higher_cutoff_freq: 10000,
                sleep_timer: 0,
                live_config: false,
            },
            input: InputConfig {
                method: InputMethod::Pipewire,
                source: "auto".to_string(),
                sample_rate: 44100,
                sample_bits: 16,
                channels: 2,
            },
            output: OutputConfig {
                method: OutputMethod::Noncurses,
                orientation: Orientation::Bottom,
                channels: 2,
                mono_option: MonoOption::Average,
                reverse: false,
                split_stereo: false,
                raw_target: "/dev/stdout".to_string(),
                data_format: DataFormat::Binary,
                bit_format: BitFormat::Bits16,
                ascii_max_range: 1000,
                bar_delimiter: 59,
                frame_delimiter: 10,
                xaxis: XAxis::None,
                show_idle_bar_heads: true,
                waveform: false,
                waveform_style: WaveformStyle::Line,
                waveform_baseline: true,
                waveform_dynamics: false,
                waveform_graticule: false,
                bar_reflection: false,
                stereo_divider: false,
                baseline_ruler: false,
                bar_cap: false,
                bright_peaks: true,
                beat_pulse: false,
                ascii_glyphs: false,
                reduce_motion: false,
                draw_peaks: true,
                peak_decay: 0.1,
            },
            color: ColorConfig {
                background: TerminalColor::Default,
                foreground: TerminalColor::Default,
                gradient: false,
                gradient_colors: vec![
                    TerminalColor::parse("#59cc33"),
                    TerminalColor::parse("#80cc33"),
                    TerminalColor::parse("#a6cc33"),
                    TerminalColor::parse("#cccc33"),
                    TerminalColor::parse("#cca633"),
                    TerminalColor::parse("#cc8033"),
                    TerminalColor::parse("#cc5933"),
                    TerminalColor::parse("#cc3333"),
                ],
                horizontal_gradient: false,
                horizontal_gradient_colors: vec![
                    TerminalColor::parse("#c45161"),
                    TerminalColor::parse("#e094a0"),
                    TerminalColor::parse("#f2b6c0"),
                    TerminalColor::parse("#f2dde1"),
                    TerminalColor::parse("#cbc7d8"),
                    TerminalColor::parse("#8db7d2"),
                    TerminalColor::parse("#5e62a9"),
                    TerminalColor::parse("#434279"),
                ],
                blend_direction: BlendDirection::Up,
                spectrum: false,
                background_gradient: false,
                theme: "none".to_string(),
            },
            smoothing: SmoothingConfig {
                monstercat: 0.0,
                waves: false,
                noise_reduction: 0.77,
                gravity: 100.0,
            },
            eq: BTreeMap::new(),
            config_path: None,
        }
    }
}

impl Config {
    pub fn locate_config_path(custom_path: Option<&Path>) -> Option<PathBuf> {
        if let Some(p) = custom_path {
            if p.exists() {
                return Some(p.to_path_buf());
            }
        }

        // Zava is config-isolated: only `zava/config` is ever loaded or saved.
        // A legacy CAVA config is deliberately NOT consumed. Two reasons:
        //   1. `save()` rewrites every key zava owns, so loading
        //      `~/.config/cava/config` means the first menu keypress rewrites
        //      (and injects zava-only keys into) the user's CAVA setup;
        //   2. CAVA's `bars`/`bar_width` are usually fixed values, which would
        //      silently switch zava out of auto-fit and stop the bars tracking
        //      the terminal size.
        // `locate_legacy_cava_path()` exists purely so the user can be warned
        // that the file is being ignored.
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let p = PathBuf::from(&xdg).join("zava/config");
            if p.exists() {
                return Some(p);
            }
        }

        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(&home).join(".config/zava/config");
            if p.exists() {
                return Some(p);
            }
        }

        None
    }

    /// Path of a legacy CAVA config, if one exists.
    ///
    /// Returned **only** so startup can tell the user it is being ignored —
    /// zava never reads or writes this file (see [`Config::locate_config_path`]).
    pub fn locate_legacy_cava_path(custom_path: Option<&Path>) -> Option<PathBuf> {
        // An explicit `-p` means the user chose the file; no warning applies.
        if custom_path.is_some() {
            return None;
        }
        if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
            let p = PathBuf::from(&xdg).join("cava/config");
            if p.exists() {
                return Some(p);
            }
        }
        if let Ok(home) = std::env::var("HOME") {
            let p = PathBuf::from(&home).join(".config/cava/config");
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    pub fn load_or_default(custom_path: Option<&Path>) -> Self {
        let mut config = Config::default();
        if let Some(path) = Self::locate_config_path(custom_path) {
            config.config_path = Some(path.clone());
            if let Ok(content) = fs::read_to_string(&path) {
                config.parse_ini(&content);
            }
        }
        config.check_theme();
        config
    }

    pub fn reload(&mut self) -> Result<(), String> {
        if let Some(path) = &self.config_path {
            let content = fs::read_to_string(path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            let mut fresh = Config::default();
            fresh.config_path = Some(path.clone());
            fresh.parse_ini(&content);
            fresh.check_theme();
            *self = fresh;
            Ok(())
        } else {
            Err("No config path set".to_string())
        }
    }

    /// Serialize the current config back to INI, preserving the existing file's
    /// comments, blank lines, unknown keys and ordering. Keys we own are
    /// rewritten in place; keys missing from the file are appended to their
    /// section. The write is atomic (sibling tmp file + rename).
    pub fn save(&mut self) -> Result<(), String> {
        // No path yet (running with defaults): create the standard user config
        // so menu changes persist from a pristine install.
        if self.config_path.is_none() {
            let home = std::env::var("HOME")
                .map_err(|_| "No config path set and HOME is not defined".to_string())?;
            let p = PathBuf::from(home).join(".config/zava/config");
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
            }
            self.config_path = Some(p);
        }
        let path = self.config_path.clone().unwrap();
        let original = fs::read_to_string(&path).unwrap_or_default();

        // Keys already present in the file, as (section, key).
        let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
        // Gradient color indices present in the file, as (horizontal, idx).
        let mut seen_grad: std::collections::HashSet<(bool, usize)> = std::collections::HashSet::new();

        let mut out_lines: Vec<String> = Vec::new();
        let mut current_section = String::new();

        for line in original.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
                out_lines.push(line.to_string());
                continue;
            }
            if trimmed.starts_with('#') || trimmed.starts_with(';') || !trimmed.contains('=') {
                out_lines.push(line.to_string());
                continue;
            }
            let (raw_key, raw_val) = trimmed.split_once('=').unwrap_or(("", ""));
            let key = raw_key.trim().to_lowercase();

            if let Some((horiz, idx)) = Self::gradient_key(&key) {
                seen_grad.insert((horiz, idx));
                let color = if horiz {
                    self.color.horizontal_gradient_colors.get(idx.wrapping_sub(1)).cloned()
                } else {
                    self.color.gradient_colors.get(idx.wrapping_sub(1)).cloned()
                };
                if let Some(c) = color {
                    let suffix = Self::ini_comment_suffix(raw_val);
                    out_lines.push(format!("{} = {}{}", key, c.to_ini(), suffix));
                    continue;
                }
            }

            if let Some(val) = self.value_of(&current_section, &key) {
                seen.insert((current_section.clone(), key.clone()));
                // Keep any trailing comment ("# frames per second" etc.) that
                // followed the old value verbatim.
                let suffix = Self::ini_comment_suffix(raw_val);
                out_lines.push(format!("{} = {}{}", key, val, suffix));
                continue;
            }
            out_lines.push(line.to_string());
        }

        // Sections and their key order, used for appending missing keys.
        const ORDER: &[(&str, &[&str])] = &[
            ("general", &[
                "framerate", "autosens", "sensitivity", "scaling", "bars", "bar_width",
                "bar_spacing", "bar_height", "center_align", "wrap_bars", "max_height",
                "lower_cutoff_freq", "higher_cutoff_freq", "sleep_timer", "live_config",
            ]),
            ("input", &[
                "method", "source", "sample_rate", "sample_bits", "channels", "autoconnect",
            ]),
            ("output", &[
                "method", "orientation", "channels", "mono_option", "reverse", "split_stereo",
                "left_bottom", "raw_target", "data_format", "bit_format", "ascii_max_range",
                "bar_delimiter", "frame_delimiter", "xaxis", "show_idle_bar_heads", "waveform",
                "waveform_style", "waveform_baseline", "waveform_dynamics", "waveform_graticule",
                "bar_reflection", "stereo_divider", "baseline_ruler", "bar_cap", "bright_peaks",
                "beat_pulse", "ascii_glyphs", "reduce_motion", "draw_peaks", "peak_decay",
            ]),
            ("color", &[
                "background", "foreground", "gradient", "gradient_count", "horizontal_gradient",
                "horizontal_gradient_count", "blend_direction", "spectrum", "background_gradient",
                "theme",
            ]),
            ("smoothing", &[
                "monstercat", "waves", "noise_reduction", "gravity",
            ]),
        ];
        // ... (appended-keys and EQ handling follow)

        // Append keys the file does not mention yet, grouped by section.
        let mut new_blocks: Vec<(String, Vec<String>)> = Vec::new();
        for (section, keys) in ORDER {
            let mut extra: Vec<String> = Vec::new();
            for k in *keys {
                if seen.contains(&((*section).to_string(), (*k).to_string())) {
                    continue;
                }
                if let Some(v) = self.value_of(section, k) {
                    extra.push(format!("{} = {}", k, v));
                }
            }
            if !extra.is_empty() {
                new_blocks.push(((*section).to_string(), extra));
            }
        }

        // Gradient colors missing from the file: attach them to the [color]
        // block (existing or newly created) so they stay in the right section.
        let mut grad_extra: Vec<String> = Vec::new();
        for (horiz, list) in [
            (false, &self.color.gradient_colors),
            (true, &self.color.horizontal_gradient_colors),
        ] {
            for (i, c) in list.iter().enumerate() {
                let idx = i + 1;
                if seen_grad.contains(&(horiz, idx)) {
                    continue;
                }
                let prefix = if horiz { "horizontal_gradient_color_" } else { "gradient_color_" };
                grad_extra.push(format!("{}{} = {}", prefix, idx, c.to_ini()));
            }
        }
        if !grad_extra.is_empty() {
            match new_blocks.iter_mut().find(|(s, _)| s == "color") {
                Some((_, lines)) => lines.extend(grad_extra),
                None => new_blocks.push(("color".to_string(), grad_extra)),
            }
        }

        // EQ entries: rewrite existing lines in place, append missing ones.
        // The EQ section uses numeric keys, handled specially above.
        let mut eq_pending: Vec<(usize, f64)> = self.eq.iter().map(|(k, v)| (*k, *v)).collect();
        if !eq_pending.is_empty() {
            // Rewrite existing eq lines in place.
            let eq_keys: std::collections::HashSet<usize> = eq_pending.iter().map(|(k, _)| *k).collect();
            for line in out_lines.iter_mut() {
                let t = line.trim();
                if t.contains('=') && !t.starts_with('#') && !t.starts_with(';') && !t.starts_with('[') {
                    if let Some((k, _)) = t.split_once('=') {
                        if let Ok(idx) = k.trim().parse::<usize>() {
                            if eq_keys.contains(&idx) {
                                if let Some(v) = self.eq.get(&idx) {
                                    *line = format!("{} = {}", idx, v);
                                    eq_pending.retain(|(k, _)| *k != idx);
                                }
                            }
                        }
                    }
                }
            }
            // If the file has no [eq] section, add one.
            if !out_lines.iter().any(|l| l.trim() == "[eq]") {
                out_lines.push(String::new());
                out_lines.push("[eq]".to_string());
            }
            // Find the index of the [eq] header and append after it.
            if let Some(pos) = out_lines.iter().position(|l| l.trim() == "[eq]") {
                let mut insert_at = pos + 1;
                while insert_at < out_lines.len()
                    && (out_lines[insert_at].trim().is_empty()
                        || out_lines[insert_at].trim().starts_with('#')
                        || (out_lines[insert_at].contains('=')
                            && !out_lines[insert_at].trim().starts_with('[')))
                {
                    insert_at += 1;
                }
                let mut block: Vec<String> = eq_pending
                    .iter()
                    .map(|(k, v)| format!("{} = {}", k, v))
                    .collect();
                block.push(String::new());
                let _ = &block;
                let tail = out_lines.split_off(insert_at);
                out_lines.extend(block);
                out_lines.extend(tail);
            }
        }

        // Merge appended keys into their section if the header already exists
        // somewhere in the file (right after that section's last key), so we
        // never emit a second [section] header. Otherwise create the header.
        for (section, lines) in new_blocks {
            let header = format!("[{}]", section);
            let mut insert_at: Option<usize> = None;
            let mut in_section = false;
            for (i, l) in out_lines.iter().enumerate() {
                let t = l.trim();
                if t.eq_ignore_ascii_case(&header) {
                    in_section = true;
                    insert_at = Some(i + 1);
                    continue;
                }
                if in_section {
                    if t.starts_with('[') && t.ends_with(']') {
                        break; // next section reached
                    }
                    if !t.is_empty() && !t.starts_with('#') && !t.starts_with(';') {
                        insert_at = Some(i + 1);
                    }
                }
            }
            match insert_at {
                Some(at) => {
                    for (off, l) in lines.iter().enumerate() {
                        out_lines.insert(at + off, l.clone());
                    }
                }
                None => {
                    out_lines.push(String::new());
                    out_lines.push(header);
                    for l in lines {
                        out_lines.push(l);
                    }
                }
            }
        }

        let mut body = out_lines.join("\n");
        body.push('\n');

        // Atomic write: sibling temp file, then rename over the target.
        let tmp = path.with_extension("zava.tmp");
        fs::write(&tmp, body)
            .map_err(|e| format!("Failed to write {}: {}", tmp.display(), e))?;
        fs::rename(&tmp, &path)
            .map_err(|e| format!("Failed to replace {}: {}", path.display(), e))?;
        Ok(())
    }

    /// Current value of a writable config key, as it should appear in the file.
    /// Returns None for keys we never write out.
    fn value_of(&self, section: &str, key: &str) -> Option<String> {
        let v = match (section, key) {
            ("general", "framerate") => self.general.framerate.to_string(),
            ("general", "autosens") => self.general.autosens.to_string(),
            ("general", "sensitivity") => format!("{:.1}", self.general.sensitivity),
            ("general", "scaling") => self.general.scaling.as_str().to_string(),
            ("general", "bars") => self.general.bars.to_string(),
            ("general", "bar_width") => self.general.bar_width.to_string(),
            ("general", "bar_spacing") => self.general.bar_spacing.to_string(),
            ("general", "center_align") => self.general.center_align.to_string(),
("general", "wrap_bars") => self.general.wrap_bars.to_string(),
            ("general", "max_height") => format!("{}", (self.general.max_height * 100.0)),
            ("general", "lower_cutoff_freq") => self.general.lower_cutoff_freq.to_string(),
            ("general", "higher_cutoff_freq") => self.general.higher_cutoff_freq.to_string(),
            ("general", "sleep_timer") => self.general.sleep_timer.to_string(),
            ("general", "live_config") => self.general.live_config.to_string(),
            ("input", "method") => self.input.method.as_str().to_string(),
            ("input", "source") => self.input.source.clone(),
            ("input", "sample_rate") => self.input.sample_rate.to_string(),
            ("input", "sample_bits") => self.input.sample_bits.to_string(),
            ("input", "channels") => self.input.channels.to_string(),
            ("output", "method") => self.output.method.as_str().to_string(),
            ("output", "orientation") => self.output.orientation.as_str().to_string(),
            ("output", "channels") => self.output.channels.to_string(),
            ("output", "mono_option") => self.output.mono_option.as_str().to_string(),
            ("output", "reverse") => self.output.reverse.to_string(),
            ("output", "split_stereo") => self.output.split_stereo.to_string(),
            ("output", "raw_target") => self.output.raw_target.clone(),
            ("output", "data_format") => self.output.data_format.as_str().to_string(),
            ("output", "bit_format") => self.output.bit_format.as_str().to_string(),
            ("output", "ascii_max_range") => self.output.ascii_max_range.to_string(),
            ("output", "bar_delimiter") => self.output.bar_delimiter.to_string(),
            ("output", "frame_delimiter") => self.output.frame_delimiter.to_string(),
            ("output", "xaxis") => self.output.xaxis.as_str().to_string(),
            ("output", "draw_peaks") => self.output.draw_peaks.to_string(),
            ("output", "peak_decay") => self.output.peak_decay.to_string(),
            ("output", "show_idle_bar_heads") => self.output.show_idle_bar_heads.to_string(),
            ("output", "waveform") => self.output.waveform.to_string(),
            ("output", "waveform_style") => self.output.waveform_style.as_str().to_string(),
            ("output", "waveform_baseline") => self.output.waveform_baseline.to_string(),
            ("output", "waveform_dynamics") => self.output.waveform_dynamics.to_string(),
            ("output", "waveform_graticule") => self.output.waveform_graticule.to_string(),
            ("output", "bar_reflection") => self.output.bar_reflection.to_string(),
            ("output", "stereo_divider") => self.output.stereo_divider.to_string(),
            ("output", "baseline_ruler") => self.output.baseline_ruler.to_string(),
            ("output", "bar_cap") => self.output.bar_cap.to_string(),
            ("output", "bright_peaks") => self.output.bright_peaks.to_string(),
            ("output", "beat_pulse") => self.output.beat_pulse.to_string(),
            ("output", "ascii_glyphs") => self.output.ascii_glyphs.to_string(),
            ("output", "reduce_motion") => self.output.reduce_motion.to_string(),
            ("color", "background") => self.color.background.to_ini(),
            ("color", "foreground") => self.color.foreground.to_ini(),
            ("color", "gradient") => self.color.gradient.to_string(),
            ("color", "horizontal_gradient") => self.color.horizontal_gradient.to_string(),
            ("color", "blend_direction") => self.color.blend_direction.as_str().to_string(),
            ("color", "spectrum") => self.color.spectrum.to_string(),
            ("color", "background_gradient") => self.color.background_gradient.to_string(),
            ("color", "theme") => self.color.theme.clone(),
            ("smoothing", "monstercat") => self.smoothing.monstercat.to_string(),
            ("smoothing", "waves") => self.smoothing.waves.to_string(),
            ("smoothing", "noise_reduction") => (self.smoothing.noise_reduction * 100.0).to_string(),
            ("smoothing", "gravity") => self.smoothing.gravity.to_string(),
            _ => return None,
        };
        Some(v)
    }

    /// Extract the trailing comment of an INI value (e.g. " 60   # frames per
    /// second" -> "   # frames per second") so save() can preserve it. A '#'
    /// or ';' only starts a comment when it sits outside quotes and there is a
    /// non-whitespace value character before it, so "#59cc33" stays a value.
    fn ini_comment_suffix(raw_val: &str) -> &str {
        let t = raw_val.trim_end();
        let mut in_quote: Option<char> = None;
        let mut saw_value_char = false;
        for (i, ch) in t.char_indices() {
            match in_quote {
                Some(q) => {
                    if ch == q {
                        in_quote = None;
                    } else {
                        saw_value_char = true;
                    }
                }
                None => {
                    if ch == '\'' || ch == '"' {
                        in_quote = Some(ch);
                    } else if (ch == '#' || ch == ';')
                        && saw_value_char
                        && i > 0
                        && t.as_bytes()[i - 1].is_ascii_whitespace()
                    {
                        // Include the whitespace run before the comment so the
                        // rewritten line keeps its column alignment.
                        let mut ws = i - 1;
                        while ws > 0 && t.as_bytes()[ws - 1].is_ascii_whitespace() {
                            ws -= 1;
                        }
                        return &t[ws..];
                    } else if !ch.is_whitespace() {
                        saw_value_char = true;
                    }
                }
            }
        }
        ""
    }

    /// Section for a gradient color key, e.g. "gradient_color_3" -> Some("color")
    /// plus the parsed index.
    fn gradient_key(key: &str) -> Option<(bool, usize)> {
        if let Some(s) = key.strip_prefix("gradient_color_") {
            s.parse::<usize>().ok().map(|i| (false, i))
        } else if let Some(s) = key.strip_prefix("horizontal_gradient_color_") {
            s.parse::<usize>().ok().map(|i| (true, i))
        } else {
            None
        }
    }

    pub fn reload_colors_only(&mut self) -> Result<(), String> {
        if let Some(path) = &self.config_path {
            let content = fs::read_to_string(path)
                .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
            let mut fresh = Config::default();
            fresh.parse_ini(&content);
            fresh.check_theme();
            self.color = fresh.color;
            Ok(())
        } else {
            Err("No config path set".to_string())
        }
    }

    fn check_theme(&mut self) {
        if self.color.theme != "none" && !self.color.theme.is_empty() {
            let theme_name = self.color.theme.clone();
            let mut candidates = Vec::new();
            if let Ok(home) = std::env::var("HOME") {
                candidates.push(PathBuf::from(&home).join(".config/cava/themes").join(&theme_name));
                candidates.push(PathBuf::from(&home).join(".config/zava/themes").join(&theme_name));
            }
            if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
                candidates.push(PathBuf::from(&xdg).join("cava/themes").join(&theme_name));
                candidates.push(PathBuf::from(&xdg).join("zava/themes").join(&theme_name));
            }
            for candidate in candidates {
                if candidate.exists() {
                    if let Ok(content) = fs::read_to_string(candidate) {
                        self.parse_ini(&content);
                        break;
                    }
                }
            }
        }
    }

    pub fn parse_ini(&mut self, content: &str) {
        let mut current_section = String::new();
        let mut gradient_colors_map: BTreeMap<usize, TerminalColor> = BTreeMap::new();
        let mut h_gradient_colors_map: BTreeMap<usize, TerminalColor> = BTreeMap::new();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
                continue;
            }

            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                current_section = trimmed[1..trimmed.len() - 1].trim().to_lowercase();
                continue;
            }

            if let Some((raw_key, raw_val)) = trimmed.split_once('=') {
                let key = raw_key.trim().to_lowercase();
                // Cut an inline comment that starts at " #" or " ;", but only
                // outside quotes, so hex colors like "#59cc33" and quoted
                // '#59cc33' survive intact; plain "#59cc33" is a value.
                let cleaned = {
                    let t = raw_val.trim();
                    let mut cut = t.len();
                    let mut in_quote: Option<char> = None;
                    for (i, ch) in t.char_indices() {
                        match in_quote {
                            Some(q) => {
                                if ch == q {
                                    in_quote = None;
                                }
                            }
                            None => {
                                if ch == '\'' || ch == '"' {
                                    in_quote = Some(ch);
                                } else if (ch == '#' || ch == ';')
                                    && i > 0
                                    && t.as_bytes()[i - 1].is_ascii_whitespace()
                                {
                                    cut = i;
                                    break;
                                }
                            }
                        }
                    }
                    &t[..cut]
                };
                let val = cleaned.trim().trim_matches('\'').trim_matches('"').trim();

                match current_section.as_str() {
                    "general" => match key.as_str() {
                        "framerate" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.general.framerate = v.clamp(1, 500);
                            }
                        }
                        "autosens" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.general.autosens = v;
                            }
                        }
                        "sensitivity" => {
                            if let Ok(v) = val.parse::<f64>() {
                                self.general.sensitivity = v;
                            }
                        }
                        "scaling" => {
                            if val.eq_ignore_ascii_case("decibel") {
                                self.general.scaling = ScalingMode::Decibel;
                            } else {
                                self.general.scaling = ScalingMode::Linear;
                            }
                        }
                        "bars" => {
                            if let Ok(v) = val.parse::<usize>() {
                                self.general.bars = v;
                            }
                        }
                        "bar_width" => {
                            if let Ok(v) = val.parse::<usize>() {
                                self.general.bar_width = v.max(1);
                            }
                        }
                        "bar_spacing" => {
                            if let Ok(v) = val.parse::<usize>() {
                                self.general.bar_spacing = v;
                            }
                        }
                        // `bar_height` is accepted for CAVA config compatibility
                        // but unused: zava sizes bars from the terminal axis.
                        "bar_height" => {}
                        "center_align" => {
                            self.general.center_align = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "wrap_bars" => {
                            self.general.wrap_bars = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "max_height" => {
                            if let Ok(v) = val.parse::<f64>() {
                                self.general.max_height = (v / 100.0).clamp(0.01, 1.0);
                            }
                        }
                        "lower_cutoff_freq" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.general.lower_cutoff_freq = v;
                            }
                        }
                        "higher_cutoff_freq" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.general.higher_cutoff_freq = v;
                            }
                        }
                        "sleep_timer" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.general.sleep_timer = v;
                            }
                        }
                        "live-config" | "live_config" => {
                            self.general.live_config = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        _ => {}
                    },
                    "input" => match key.as_str() {
                        "method" => match val.to_lowercase().as_str() {
                            "pipewire" => self.input.method = InputMethod::Pipewire,
                            "pulse" => self.input.method = InputMethod::Pulse,
                            "alsa" => self.input.method = InputMethod::Alsa,
                            "fifo" => self.input.method = InputMethod::Fifo,
                            "cpal" | "portaudio" | "coreaudio" => self.input.method = InputMethod::Cpal,
                            "test" => self.input.method = InputMethod::Test,
                            _ => {}
                        },
                        "source" => {
                            self.input.source = val.to_string();
                        }
                        "sample_rate" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.input.sample_rate = v;
                            }
                        }
                        "sample_bits" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.input.sample_bits = v;
                            }
                        }
                        "channels" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.input.channels = v.clamp(1, 2);
                            }
                        }
                        // `autoconnect` is accepted for CAVA config compatibility but
                        // unused: input backends pick their own device.
                        "autoconnect" => {}
                        _ => {}
                    },
                    "output" => match key.as_str() {
                        "method" => match val.to_lowercase().as_str() {
                            "noncurses" => self.output.method = OutputMethod::Noncurses,
                            "ncurses" => self.output.method = OutputMethod::Ncurses,
                            "raw" => self.output.method = OutputMethod::Raw,
                            _ => {}
                        },
                        "orientation" => match val.to_lowercase().as_str() {
                            "bottom" => self.output.orientation = Orientation::Bottom,
                            "top" => self.output.orientation = Orientation::Top,
                            "left" => self.output.orientation = Orientation::Left,
                            "right" => self.output.orientation = Orientation::Right,
                            "horizontal" => self.output.orientation = Orientation::Horizontal,
                            // `vertical` was removed: the mirrored centre-column
                            // mode needed rare already-half-width glyphs and only
                            // ever used half the axis. Existing configs fall back
                            // to the classic bottom-up layout.
                            "vertical" => self.output.orientation = Orientation::Bottom,
                            _ => {}
                        },
                        "channels" => {
                            if val.eq_ignore_ascii_case("mono") || val == "1" {
                                self.output.channels = 1;
                            } else {
                                self.output.channels = 2;
                            }
                        }
                        "mono_option" => match val.to_lowercase().as_str() {
                            "left" => self.output.mono_option = MonoOption::Left,
                            "right" => self.output.mono_option = MonoOption::Right,
                            _ => self.output.mono_option = MonoOption::Average,
                        },
                        "reverse" => {
                            self.output.reverse = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "split_stereo" => {
                            self.output.split_stereo = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        // `left_bottom` is accepted for CAVA config compatibility but
                        // unused: the bottom orientation always fills upward.
                        "left_bottom" => {}
                        "raw_target" => {
                            self.output.raw_target = val.to_string();
                        }
                        "data_format" => {
                            if val.eq_ignore_ascii_case("ascii") {
                                self.output.data_format = DataFormat::Ascii;
                            } else {
                                self.output.data_format = DataFormat::Binary;
                            }
                        }
                        "bit_format" => {
                            if val.eq_ignore_ascii_case("8bit") {
                                self.output.bit_format = BitFormat::Bits8;
                            } else {
                                self.output.bit_format = BitFormat::Bits16;
                            }
                        }
                        "ascii_max_range" => {
                            if let Ok(v) = val.parse::<u32>() {
                                self.output.ascii_max_range = v;
                            }
                        }
                        "bar_delimiter" => {
                            if let Ok(v) = val.parse::<u8>() {
                                self.output.bar_delimiter = v;
                            }
                        }
                        "frame_delimiter" => {
                            if let Ok(v) = val.parse::<u8>() {
                                self.output.frame_delimiter = v;
                            }
                        }
                        "xaxis" => {
                            if val.eq_ignore_ascii_case("frequency") {
                                self.output.xaxis = XAxis::Frequency;
                            } else {
                                self.output.xaxis = XAxis::None;
                            }
                        }
                        "show_idle_bar_heads" => {
                            self.output.show_idle_bar_heads = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "waveform" => {
                            self.output.waveform = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "waveform_style" => {
                            self.output.waveform_style = if val.eq_ignore_ascii_case("area") {
                                WaveformStyle::Area
                            } else {
                                WaveformStyle::Line
                            };
                        }
                        "waveform_baseline" => {
                            self.output.waveform_baseline =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "waveform_dynamics" => {
                            self.output.waveform_dynamics =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "waveform_graticule" => {
                            self.output.waveform_graticule =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "bar_reflection" => {
                            self.output.bar_reflection =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "stereo_divider" => {
                            self.output.stereo_divider =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "baseline_ruler" => {
                            self.output.baseline_ruler =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "bar_cap" => {
                            self.output.bar_cap = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "bright_peaks" => {
                            self.output.bright_peaks =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "beat_pulse" => {
                            self.output.beat_pulse =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "ascii_glyphs" => {
                            self.output.ascii_glyphs =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "reduce_motion" => {
                            self.output.reduce_motion =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "draw_peaks" => {
                            self.output.draw_peaks = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "peak_decay" => {
                            if let Ok(v) = val.parse::<f64>() {
                                self.output.peak_decay = v.clamp(0.0, 10.0);
                            }
                        }
                        _ => {}
                    },
                    "color" => {
                        if key == "background" {
                            self.color.background = TerminalColor::parse(val);
                        } else if key == "foreground" {
                            self.color.foreground = TerminalColor::parse(val);
                        } else if key == "gradient" {
                            self.color.gradient = val == "1" || val.eq_ignore_ascii_case("true");
                        // `gradient_count` is accepted for CAVA config compatibility but
                        // unused: the number of gradient colors is the length
                        // of the gradient_color_N list itself.
                        } else if key == "gradient_count" {
                        } else if let Some(stripped) = key.strip_prefix("gradient_color_") {
                            if let Ok(idx) = stripped.parse::<usize>() {
                                gradient_colors_map.insert(idx, TerminalColor::parse(val));
                            }
                        } else if key == "horizontal_gradient" {
                            self.color.horizontal_gradient = val == "1" || val.eq_ignore_ascii_case("true");
                        // `horizontal_gradient_count` is accepted for CAVA config
                        // compatibility but unused (see `gradient_count`).
                        } else if key == "horizontal_gradient_count" {
                        } else if let Some(stripped) = key.strip_prefix("horizontal_gradient_color_") {
                            if let Ok(idx) = stripped.parse::<usize>() {
                                h_gradient_colors_map.insert(idx, TerminalColor::parse(val));
                            }
                        } else if key == "blend_direction" {
                            match val.to_lowercase().as_str() {
                                "down" => self.color.blend_direction = BlendDirection::Down,
                                "left" => self.color.blend_direction = BlendDirection::Left,
                                "right" => self.color.blend_direction = BlendDirection::Right,
                                _ => self.color.blend_direction = BlendDirection::Up,
                            }
                        } else if key == "spectrum" {
                            self.color.spectrum = val == "1" || val.eq_ignore_ascii_case("true");
                        } else if key == "background_gradient" {
                            self.color.background_gradient =
                                val == "1" || val.eq_ignore_ascii_case("true");
                        } else if key == "theme" {
                            self.color.theme = val.to_string();
                        }
                    }
                    "smoothing" => match key.as_str() {
                        "monstercat" => {
                            if let Ok(v) = val.parse::<f64>() {
                                self.smoothing.monstercat = v;
                            }
                        }
                        "waves" => {
                            self.smoothing.waves = val == "1" || val.eq_ignore_ascii_case("true");
                        }
                        "noise_reduction" => {
                            if let Ok(v) = val.parse::<f64>() {
                                self.smoothing.noise_reduction = (v / 100.0).clamp(0.0, 1.0);
                            }
                        }
                        "gravity" => {
                            if let Ok(v) = val.parse::<f64>() {
                                self.smoothing.gravity = v.clamp(0.0, 500.0);
                            }
                        }
                        _ => {}
                    },
                    "eq" => {
                        if let Ok(idx) = key.parse::<usize>() {
                            if let Ok(gain) = val.parse::<f64>() {
                                // CAVA semantics: [eq] values are raw linear
                                // multipliers, not decibels. A negative gain
                                // would mirror the bar below the baseline (and
                                // render as nothing), so floor it at zero.
                                if gain.is_finite() {
                                    self.eq.insert(idx, gain.max(0.0));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        if !gradient_colors_map.is_empty() {
            let mut list = Vec::new();
            for i in 1..=8 {
                if let Some(c) = gradient_colors_map.get(&i) {
                    list.push(c.clone());
                }
            }
            if !list.is_empty() {
                self.color.gradient_colors = list;
            }
        }

        if !h_gradient_colors_map.is_empty() {
            let mut list = Vec::new();
            for i in 1..=8 {
                if let Some(c) = h_gradient_colors_map.get(&i) {
                    list.push(c.clone());
                }
            }
            if !list.is_empty() {
                self.color.horizontal_gradient_colors = list;
            }
        }
    }
}

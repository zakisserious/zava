use crate::config::{BlendDirection, ColorConfig, RgbColor, TerminalColor};

/// True when the terminal advertises 24-bit colour support, read once from
/// `COLORTERM`. Defaults to true (vivid RGB) when the variable is unset —
/// every modern terminal supports it and the fallback only engages when a
/// terminal explicitly says it does not.
fn detect_truecolor() -> bool {
    match std::env::var("COLORTERM") {
        Ok(v) => {
            let v = v.to_ascii_lowercase();
            v.contains("truecolor") || v.contains("24bit")
        }
        Err(_) => true,
    }
}

/// Map an sRGB triple to the nearest xterm-256 index for terminals without
/// truecolor support (a 6x6x6 colour cube offset into the ANSI palette).
fn color256(r: u8, g: u8, b: u8) -> u8 {
    let c6 = |v: u8| -> u16 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((v as u16 - 35) / 40).min(5)
        }
    };
    (16 + 36 * c6(r) + 6 * c6(g) + c6(b)) as u8
}

/// Convert an HSL colour to sRGB. `h` in degrees [0, 360); `s`, `l` in [0, 1].
fn hsl(h: f64, s: f64, l: f64) -> (u8, u8, u8) {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let hp = ((h % 360.0 + 360.0) % 360.0) / 60.0;
    let x = c * (1.0 - (hp % 2.0 - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
}

#[derive(Debug, Clone)]
pub struct ColorMap {
    // 2D grid of RGB colors [line][bar]
    pub table: Vec<Vec<RgbColor>>,
    pub fg_ansi: String,
    pub bg_ansi: String,
    pub has_gradient: bool,
    truecolor: bool,
    /// High-contrast ink (white on dark, black on light foregrounds) used for
    /// peak caps and beat/highlight accents.
    accent_ansi: String,
    fg_rgb: Option<RgbColor>,
    spectrum: bool,
    /// Per-line background escapes when `background_gradient` is active (a dim
    /// vertical ramp); empty when flat.
    bg_rows: Vec<String>,
}

impl ColorMap {
    pub fn new(config: &ColorConfig, lines: usize, bars: usize) -> Self {
        Self::new_tc(config, lines, bars, detect_truecolor())
    }

    /// Build with an explicit truecolor flag (tests pin this so escape
    /// assertions do not depend on the ambient `COLORTERM`).
    pub(crate) fn new_tc(
        config: &ColorConfig,
        lines: usize,
        bars: usize,
        truecolor: bool,
    ) -> Self {
        let lines = lines.max(1);
        let bars = bars.max(1);

        let fg_ansi = Self::color_to_fg_ansi(&config.foreground, truecolor);
        let bg_ansi = Self::color_to_bg_ansi(&config.background, truecolor);
        let fg_rgb = config.foreground.to_rgb();

        let accent_ansi = match &fg_rgb {
            Some(c) if (c.r as u16 + c.g as u16 + c.b as u16) < 450 => {
                Self::rgb_fg_rgb(255, 255, 255, truecolor)
            }
            _ => Self::rgb_fg_rgb(0, 0, 0, truecolor),
        };

        // Vertical (per-line) and horizontal (per-bar) gradient ramps share one
        // interpolation pass; an empty ramp degrades to a blank white fill.
        let v_colors = if config.gradient && !config.gradient_colors.is_empty() {
            Self::interpolate_colors(&config.gradient_colors, lines)
        } else {
            Vec::new()
        };
        let h_colors = if config.horizontal_gradient && !config.horizontal_gradient_colors.is_empty()
        {
            Self::interpolate_colors(&config.horizontal_gradient_colors, bars)
        } else {
            Vec::new()
        };

        let has_gradient =
            config.spectrum || config.gradient || config.horizontal_gradient;
        let fg = config.foreground.to_rgb();
        let white = || RgbColor { r: 255, g: 255, b: 255 };

        let cell = |y: usize, x: usize| -> RgbColor {
            match (v_colors.get(y), h_colors.get(x)) {
                (Some(v), Some(h)) => {
                    let current_height = y as f64 / lines as f64;
                    let current_width = x as f64 / bars as f64;
                    let (w_v, w_h) = match config.blend_direction {
                        BlendDirection::Up => (current_height, 1.0 - current_height),
                        BlendDirection::Down => (1.0 - current_height, current_height),
                        BlendDirection::Left => (1.0 - current_width, current_width),
                        BlendDirection::Right => (current_width, 1.0 - current_width),
                    };
                    RgbColor {
                        r: (v.r as f64 * w_v + h.r as f64 * w_h).clamp(0.0, 255.0) as u8,
                        g: (v.g as f64 * w_v + h.g as f64 * w_h).clamp(0.0, 255.0) as u8,
                        b: (v.b as f64 * w_v + h.b as f64 * w_h).clamp(0.0, 255.0) as u8,
                    }
                }
                (Some(v), None) => v.clone(),
                (None, Some(h)) => h.clone(),
                (None, None) => fg.clone().unwrap_or_else(white),
            }
        };
        let mut table = vec![vec![white(); bars]; lines];
        for y in 0..lines {
            for x in 0..bars {
                table[y][x] = cell(y, x);
            }
        }

        // Background ramp: a dim gradient of the foreground hue (or a neutral
        // slate when the foreground is unset), darkest at the bottom line.
        let bg_rows = if config.background_gradient {
            let base = fg_rgb.as_ref().map(|c| (c.r, c.g, c.b)).unwrap_or((58, 68, 88));
            (0..lines)
                .map(|y| {
                    let f = 0.10 + 0.30 * (y as f64 / lines as f64);
                    Self::rgb_bg_rgb(
                        (base.0 as f64 * f) as u8,
                        (base.1 as f64 * f) as u8,
                        (base.2 as f64 * f) as u8,
                        truecolor,
                    )
                })
                .collect()
        } else {
            Vec::new()
        };

        Self {
            table,
            fg_ansi,
            bg_ansi,
            has_gradient,
            truecolor,
            accent_ansi,
            fg_rgb,
            spectrum: config.spectrum,
            bg_rows,
        }
    }

    /// The gradient RGB at (line, bar), when a gradient is active; used to
    /// style individual spans. `None` means no gradient → caller should fall
    /// back to the configured foreground.
    pub(crate) fn color_at(&self, y: usize, x: usize) -> Option<RgbColor> {
        if self.spectrum {
            let bars = self.table.first().map(|r| r.len()).unwrap_or(1).max(1);
            let lines = self.table.len().max(1);
            let t = (x as f64 / bars as f64).clamp(0.0, 1.0);
            let hue = t * 285.0; // low = warm red, high = violet
            let light = 0.42 + 0.34 * (y as f64 / lines as f64);
            let (r, g, b) = hsl(hue, 0.85, light);
            return Some(RgbColor { r, g, b });
        }
        if !self.has_gradient {
            return None;
        }
        self.table.get(y)?.get(x).cloned()
    }

    pub fn get_fg_escape(&self, y: usize, x: usize) -> String {
        if let Some(c) = self.color_at(y, x) {
            return Self::rgb_fg_rgb(c.r, c.g, c.b, self.truecolor);
        }
        self.fg_ansi.clone()
    }

    /// A dimmed copy of the (vertical) gradient colour at (y, x), for shadows
    /// and reflected copies of the spectrum.
    pub(crate) fn dim_fg(&self, y: usize, x: usize, factor: f64) -> String {
        let rgb = self
            .color_at(y, x)
            .or_else(|| self.fg_rgb.clone())
            .unwrap_or(RgbColor { r: 90, g: 90, b: 90 });
        Self::rgb_fg_rgb(
            (rgb.r as f64 * factor) as u8,
            (rgb.g as f64 * factor) as u8,
            (rgb.b as f64 * factor) as u8,
            self.truecolor,
        )
    }

    /// High-contrast accent ink (white on dark foregrounds, black on light).
    pub(crate) fn accent_escape(&self) -> &str {
        &self.accent_ansi
    }

    /// The background escape for line `y` of a vertical background ramp.
    pub(crate) fn bg_row(&self, y: usize) -> Option<&str> {
        self.bg_rows.get(y).map(String::as_str)
    }

    fn rgb_fg_rgb(r: u8, g: u8, b: u8, truecolor: bool) -> String {
        if truecolor {
            format!("\x1b[38;2;{r};{g};{b}m")
        } else {
            format!("\x1b[38;5;{}m", color256(r, g, b))
        }
    }

    fn rgb_bg_rgb(r: u8, g: u8, b: u8, truecolor: bool) -> String {
        if truecolor {
            format!("\x1b[48;2;{r};{g};{b}m")
        } else {
            format!("\x1b[48;5;{}m", color256(r, g, b))
        }
    }

    fn interpolate_colors(defs: &[TerminalColor], count: usize) -> Vec<RgbColor> {
        let rgbs: Vec<RgbColor> = defs.iter().filter_map(|c| c.to_rgb()).collect();

        if rgbs.is_empty() {
            return vec![RgbColor { r: 255, g: 255, b: 255 }; count];
        }
        if rgbs.len() == 1 || count <= 1 {
            return vec![rgbs[0].clone(); count];
        }

        let num_segments = rgbs.len() - 1;
        let mut result = Vec::with_capacity(count);

        for i in 0..count {
            let t = (i as f64 / (count - 1) as f64) * num_segments as f64;
            let seg = (t.floor() as usize).min(num_segments - 1);
            let frac = t - seg as f64;

            let c0 = &rgbs[seg];
            let c1 = &rgbs[seg + 1];

            let r = (c0.r as f64 + (c1.r as f64 - c0.r as f64) * frac).round() as u8;
            let g = (c0.g as f64 + (c1.g as f64 - c0.g as f64) * frac).round() as u8;
            let b = (c0.b as f64 + (c1.b as f64 - c0.b as f64) * frac).round() as u8;

            result.push(RgbColor { r, g, b });
        }

        result
    }

    fn color_to_fg_ansi(c: &TerminalColor, truecolor: bool) -> String {
        // Named colours render as their vivid RGB so the app, the menu preview
        // and the menu swatch all agree (`Default` stays the uncoloured
        // `\x1b[39m` reset; background colours keep their ANSI set as-is).
        if let TerminalColor::Default = c {
            return "\x1b[39m".to_string();
        }
        let rgb = c.to_rgb().expect("all named terminal colours map to RGB");
        Self::rgb_fg_rgb(rgb.r, rgb.g, rgb.b, truecolor)
    }

    fn color_to_bg_ansi(c: &TerminalColor, truecolor: bool) -> String {
        match c {
            TerminalColor::Default => "\x1b[49m".to_string(),
            TerminalColor::Black => "\x1b[40m".to_string(),
            TerminalColor::Red => "\x1b[41m".to_string(),
            TerminalColor::Green => "\x1b[42m".to_string(),
            TerminalColor::Yellow => "\x1b[43m".to_string(),
            TerminalColor::Blue => "\x1b[44m".to_string(),
            TerminalColor::Magenta => "\x1b[45m".to_string(),
            TerminalColor::Cyan => "\x1b[46m".to_string(),
            TerminalColor::White => "\x1b[47m".to_string(),
            TerminalColor::Rgb(r, g, b) => Self::rgb_bg_rgb(*r, *g, *b, truecolor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ColorConfig;

    fn cfg(fg: TerminalColor) -> ColorConfig {
        ColorConfig {
            background: TerminalColor::Default,
            foreground: fg,
            gradient: false,
            gradient_colors: Vec::new(),
            horizontal_gradient: false,
            horizontal_gradient_colors: Vec::new(),
            blend_direction: crate::config::BlendDirection::Up,
            spectrum: false,
            background_gradient: false,
            theme: String::new(),
        }
    }

    /// With no gradient the flat renderer must emit the foreground's vivid
    /// 24-bit RGB, not the terminal-theme ANSI escape, so the app matches the
    /// menu preview and the menu swatch.
    #[test]
    fn no_gradient_fg_escape_is_vivid_rgb() {
        let cmap = ColorMap::new_tc(&cfg(TerminalColor::Red), 4, 8, true);
        assert_eq!(cmap.get_fg_escape(2, 2), "\x1b[38;2;255;0;0m");

        let cyan = ColorMap::new_tc(&cfg(TerminalColor::Cyan), 4, 8, true);
        assert_eq!(cyan.get_fg_escape(0, 7), "\x1b[38;2;0;255;255m");

        let rgb = ColorMap::new_tc(&cfg(TerminalColor::Rgb(12, 34, 56)), 4, 8, true);
        assert_eq!(rgb.get_fg_escape(0, 0), "\x1b[38;2;12;34;56m");
    }

    #[test]
    fn default_fg_stays_uncoloured() {
        let cmap = ColorMap::new_tc(&cfg(TerminalColor::Default), 4, 8, true);
        assert_eq!(cmap.get_fg_escape(1, 1), "\x1b[39m");
    }

    /// A terminal without truecolor support falls back to the nearest xterm
    /// 256-colour cube index instead of emitting a 24-bit escape.
    #[test]
    fn non_truecolor_falls_back_to_256_cube() {
        let cmap = ColorMap::new_tc(&cfg(TerminalColor::Red), 4, 8, false);
        assert_eq!(cmap.get_fg_escape(2, 2), "\x1b[38;5;196m");
    }

    #[test]
    fn gradient_overrides_the_fg_escape() {
        let mut c = cfg(TerminalColor::Red);
        c.gradient = true;
        c.gradient_colors = vec![TerminalColor::Black, TerminalColor::White];
        let cmap = ColorMap::new_tc(&c, 8, 4, true);
        assert!(cmap.get_fg_escape(0, 0).starts_with("\x1b[38;2;"));
        assert_ne!(cmap.get_fg_escape(0, 0), cmap.get_fg_escape(7, 0));
    }

    /// Spectrum mode colours by bar index (hue sweep), not by palette, and
    /// changes down the line axis (brighter toward the top).
    #[test]
    fn spectrum_mode_sweeps_hue_across_bars() {
        let mut c = cfg(TerminalColor::Red);
        c.spectrum = true;
        let cmap = ColorMap::new_tc(&c, 8, 32, true);
        assert!(cmap.has_gradient);
        let low = cmap.color_at(4, 0).unwrap();
        let high = cmap.color_at(4, 31).unwrap();
        assert_ne!((low.r, low.g, low.b), (high.r, high.g, high.b));
        // Warm red left, cool violet right: red channel strongest at the left.
        assert!(
            low.r >= low.b,
            "leftmost bar must be warm: got r={} b={}",
            low.r,
            low.b
        );
        assert!(
            high.b >= high.r,
            "rightmost bar must be cool: got r={} b={}",
            high.r,
            high.b
        );
    }

    /// The accent ink must invert against bright/dark foregrounds so it stays
    /// readable on both.
    #[test]
    fn accent_contrasts_foreground() {
        let dark = ColorMap::new_tc(&cfg(TerminalColor::Blue), 4, 8, true);
        assert_eq!(dark.accent_escape(), "\x1b[38;2;255;255;255m");
        let light = ColorMap::new_tc(&cfg(TerminalColor::White), 4, 8, true);
        assert_eq!(light.accent_escape(), "\x1b[38;2;0;0;0m");
    }

    /// The background ramp emits one per-line escape, dimmest at the bottom.
    #[test]
    fn background_gradient_gives_per_row_escapes() {
        let mut c = cfg(TerminalColor::Default);
        c.background_gradient = true;
        let cmap = ColorMap::new_tc(&c, 8, 4, true);
        assert!(cmap.bg_row(0).is_some());
        assert!(cmap.bg_row(7).is_some());
        assert_ne!(cmap.bg_row(0).unwrap(), cmap.bg_row(7).unwrap());

        let flat = ColorMap::new_tc(&cfg(TerminalColor::Default), 8, 4, true);
        assert!(flat.bg_row(3).is_none());
    }

    #[test]
    fn k256_cube_maps_primary_colours() {
        assert_eq!(color256(255, 0, 0), 196);
        assert_eq!(color256(0, 255, 0), 46);
        assert_eq!(color256(0, 0, 255), 21);
        assert_eq!(color256(0, 0, 0), 16);
        assert_eq!(color256(255, 255, 255), 231);
    }
}
use crate::config::{Config, InputMethod, Orientation, OutputMethod, TerminalColor};
use crate::menu_preview::{render_preview, to_lines, Preview};
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, Clear, ClearType};
use std::io::IsTerminal;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, List, ListItem, ListState},
    Terminal, TerminalOptions, Viewport,
};

/// Largest fixed bar count that still fits the current terminal. Mirrors the
/// layout guarantee in `render/terminal.rs`: every bar stays inside the axis,
/// so an over-eager count can no longer wrap onto the next line and make the
/// bars look like they overlap.
fn max_bars_that_fit(config: &Config) -> usize {
    bars_that_fit(dim_for(config), config)
}

/// Terminal dimension the bars are laid out along: rows for the vertical
/// (left/right) orientations, columns for everything else.
fn dim_for(config: &Config) -> usize {
    let (w, h) = crossterm::terminal::size().unwrap_or((80, 24));
    let (w, h) = (w as usize, h as usize);
    match config.output.orientation {
        Orientation::Left | Orientation::Right => h,
        _ => w,
    }
}

/// Bars that fit an axis of `dim` cells, kept even in stereo so the two channel
/// halves stay symmetric. Uses the *effective* bar width, so the menu never
/// lets the count disagree with what the renderer actually draws.
fn bars_that_fit(dim: usize, config: &Config) -> usize {
    let fit = crate::render::bars_that_fit(
        dim,
        crate::render::effective_bar_width(config, dim),
        config.general.bar_spacing,
    );
    if config.output.channels == 2 {
        fit - fit % 2
    } else {
        fit
    }
}

/// Widest bar (capped at 64) that still leaves a usable number of bars on
/// screen. Mirrors `wrap_bar_width` in `main.rs` so the menu and the arrow keys
/// agree on where the auto-mode range ends.
fn wrap_bar_width(dim: usize, spacing: usize) -> usize {
    crate::render::widest_sane_bar_width(dim, spacing)
}

/// Output methods offered while stdout is a TTY.
///
/// `raw` writes a binary data stream to stdout, which is meaningless when the
/// menu itself is being drawn on that terminal — and since every menu
/// adjustment is saved immediately, choosing it there bricked the display on
/// the next launch. It is therefore not offered as a cyclable choice while
/// stdout is a TTY (a `--output raw` invocation still works for piping).
fn output_methods() -> Vec<OutputMethod> {
    if std::io::stdout().is_terminal() {
        vec![OutputMethod::Noncurses, OutputMethod::Ncurses]
    } else {
        vec![
            OutputMethod::Noncurses,
            OutputMethod::Ncurses,
            OutputMethod::Raw,
        ]
    }
}

/// Cyclable gradient palettes for the "Gradient Palette" row. Stored as raw RGB
/// tuples so the table can be a const; cycling writes the 8 colors into
/// `config.color.gradient_colors`.
const GRADIENTS: &[(&str, [(u8, u8, u8); 8])] = &[
    ("cava", [
        (0x59, 0xcc, 0x33), (0x80, 0xcc, 0x33), (0xa6, 0xcc, 0x33),
        (0xcc, 0xcc, 0x33), (0xcc, 0xa6, 0x33), (0xcc, 0x80, 0x33),
        (0xcc, 0x59, 0x33), (0xcc, 0x33, 0x33),
    ]),
    ("fire", [
        (0x2a, 0x04, 0x04), (0x55, 0x0a, 0x00), (0x8a, 0x1c, 0x00),
        (0xb4, 0x2e, 0x00), (0xd9, 0x4a, 0x10), (0xf2, 0x68, 0x18),
        (0xff, 0x8c, 0x3c), (0xff, 0xd2, 0x9e),
    ]),
    ("ocean", [
        (0x00, 0x3b, 0x6f), (0x00, 0x59, 0x95), (0x00, 0x73, 0xb7),
        (0x16, 0x87, 0xc1), (0x3a, 0xa0, 0xd1), (0x64, 0xb8, 0xde),
        (0x8f, 0xd0, 0xea), (0xbf, 0xe7, 0xf5),
    ]),
    ("lush", [
        (0x0a, 0x4d, 0x2c), (0x1a, 0x6b, 0x3a), (0x2e, 0x8a, 0x4c),
        (0x4c, 0xa8, 0x61), (0x6f, 0xc2, 0x79), (0x95, 0xd9, 0x96),
        (0xbb, 0xeb, 0xae), (0xe4, 0xf8, 0xd4),
    ]),
    ("neon", [
        (0xff, 0x3d, 0x81), (0xff, 0x6a, 0x5e), (0xff, 0xd1, 0x66),
        (0x06, 0xd6, 0xa0), (0x11, 0x8a, 0xb2), (0x0b, 0x5f, 0x9e),
        (0x5e, 0x60, 0xce), (0xbc, 0x4e, 0xd8),
    ]),
    ("twilight", [
        (0xc4, 0x51, 0x61), (0xe0, 0x94, 0xa0), (0xf2, 0xb6, 0xc0),
        (0xf2, 0xdd, 0xe1), (0xcb, 0xc7, 0xd8), (0x8d, 0xb7, 0xd2),
        (0x5e, 0x62, 0xa9), (0x43, 0x42, 0x79),
    ]),
];

fn palette_colors(p: &[(u8, u8, u8); 8]) -> Vec<TerminalColor> {
    p.iter().map(|&(r, g, b)| TerminalColor::Rgb(r, g, b)).collect()
}

fn current_gradient_idx(config: &Config) -> usize {
    GRADIENTS
        .iter()
        .position(|(_, colors)| config.color.gradient_colors == palette_colors(colors))
        .unwrap_or(0)
}

const FG_COLORS: [TerminalColor; 8] = [
    TerminalColor::Default,
    TerminalColor::White,
    TerminalColor::Red,
    TerminalColor::Green,
    TerminalColor::Yellow,
    TerminalColor::Blue,
    TerminalColor::Magenta,
    TerminalColor::Cyan,
];
const BG_COLORS: [TerminalColor; 5] = [
    TerminalColor::Default,
    TerminalColor::Black,
    TerminalColor::Blue,
    TerminalColor::Cyan,
    TerminalColor::White,
];
const INPUT_METHODS: [InputMethod; 6] = [
    InputMethod::Pipewire,
    InputMethod::Pulse,
    InputMethod::Alsa,
    InputMethod::Fifo,
    InputMethod::Cpal,
    InputMethod::Test,
];

fn terminal_color(c: &TerminalColor) -> Color {
    match c {
        TerminalColor::Default => Color::Reset,
        TerminalColor::Black => Color::Black,
        TerminalColor::Red => Color::Red,
        TerminalColor::Green => Color::Green,
        TerminalColor::Yellow => Color::Yellow,
        TerminalColor::Blue => Color::Blue,
        TerminalColor::Magenta => Color::Magenta,
        TerminalColor::Cyan => Color::Cyan,
        TerminalColor::White => Color::White,
        TerminalColor::Rgb(r, g, b) => Color::Rgb(*r, *g, *b),
    }
}

/// Which way a Left/Right press moves a row.
#[derive(Clone, Copy)]
enum Dir {
    Inc,
    Dec,
}

impl Dir {
    fn step(&self, v: f64) -> f64 {
        match self {
            Dir::Inc => v,
            Dir::Dec => -v,
        }
    }

    fn step_usize(&self, v: usize) -> isize {
        match self {
            Dir::Inc => v as isize,
            Dir::Dec => -(v as isize),
        }
    }
}

/// Step an element of `list` by one in `dir`, wrapping. Falls back to the
/// first element when `cur` is not in `list` (custom value, e.g. edited fg).
fn cycle<T: PartialEq + Clone>(list: &[T], cur: &T, dir: Dir) -> T {
    let n = list.len();
    if n == 0 {
        return cur.clone();
    }
    let idx = list.iter().position(|x| x == cur).unwrap_or(0);
    let step = match dir {
        Dir::Inc => 1,
        Dir::Dec => n - 1,
    };
    list[(idx + step) % n].clone()
}

fn onoff(v: bool) -> &'static str {
    if v {
        "ON"
    } else {
        "OFF"
    }
}

/// One menu row: how it is drawn, how Left/Right adjust it, how `r` restores
/// it to its own default, and (numeric rows only) how a typed value is applied.
struct Row {
    label: fn(&Config) -> Line<'static>,
    adjust: fn(&mut Config, Dir),
    reset: fn(&mut Config),
    /// Applies a directly typed value, clamped exactly like `adjust` would
    /// clamp its steps. `None` on rows that have no single numeric value
    /// (toggles, cycles, colours), which are therefore not type-editable.
    set: Option<fn(&mut Config, f64)>,
}

fn label(name: &str, value: String) -> Line<'static> {
    Line::from(format!("{name:<20}{value:>10}"))
}

fn mono(value: &TerminalColor) -> String {
    value.to_ini()
}

const ROWS: &[Row] = &[
    Row {
        label: |c| label("Sensitivity", format!("{:>8.1}", c.general.sensitivity)),
        adjust: |c, d| {
            c.general.sensitivity = (c.general.sensitivity + d.step(5.0)).clamp(1.0, 500.0)
        },
        reset: |c| c.general.sensitivity = 100.0,
        set: Some(|c, v| c.general.sensitivity = v.clamp(1.0, 500.0)),
    },
    Row {
        label: |c| label("Autosens", format!("{:>8}", onoff(c.general.autosens != 0))),
        adjust: |c, _| c.general.autosens = if c.general.autosens != 0 { 0 } else { 1 },
        reset: |c| c.general.autosens = 1,
        set: None,
    },
    Row {
        label: |c| {
            let v = if c.general.bars == 0 {
                "auto".to_string()
            } else {
                c.general.bars.to_string()
            };
            label("Bars", v)
        },
        // CAVA semantics: in auto mode (bars=0) the arrows change bar_width so
        // bars fill the terminal; they never jump to a fixed count that could
        // overflow the width and make bars overlap.
        adjust: |c, d| {
            let dim = dim_for(c);
            let max_w = wrap_bar_width(dim, c.general.bar_spacing);
            let cap = max_bars_that_fit(c);
            if c.general.bars == 0 {
                let current = crate::render::effective_bar_width(c, dim);
                match d {
                    Dir::Inc => {
                        if current >= max_w {
                            c.general.bar_width = if c.general.wrap_bars { 1 } else { max_w };
                        } else {
                            c.general.bar_width = current + 1;
                        }
                    }
                    Dir::Dec => {
                        if current <= 1 {
                            if c.general.wrap_bars {
                                c.general.bar_width = max_w;
                            }
                        } else {
                            c.general.bar_width = current - 1;
                        }
                    }
                }
            } else {
                match d {
                    Dir::Inc => {
                        if c.general.bars >= cap && c.general.wrap_bars {
                            c.general.bars = 1;
                        } else {
                            c.general.bars = (c.general.bars + 1).min(cap);
                        }
                    }
                    Dir::Dec => {
                        if c.general.bars <= 1 && c.general.wrap_bars {
                            c.general.bars = max_bars_that_fit(c).max(1);
                        } else {
                            c.general.bars = c.general.bars.saturating_sub(1).max(1);
                        }
                    }
                }
            }
        },
        // Reset the whole auto-fit trio the row's arrows juggle: a bare
        // `bars = 0` would leave an over-wide bar_width stuck in auto mode.
        reset: |c| {
            c.general.bars = 0;
            c.general.bar_width = 2;
            c.general.bar_spacing = 1;
        },
        // 0 (or negative) means auto, matching the label; a positive count is
        // capped at what fits so a typed value cannot overflow the axis.
        set: Some(|c, v| {
            let n = v.round();
            if n <= 0.0 {
                c.general.bars = 0;
            } else {
                c.general.bars = (n as usize).min(max_bars_that_fit(c)).max(1);
            }
        }),
    },
    Row {
        label: |c| {
            let v = if c.general.bars == 0 {
                crate::render::effective_bar_width(c, dim_for(c))
            } else {
                c.general.bar_width
            };
            label("Bar Width", format!("{v:>8}"))
        },
        // Clamped to the widest bar that still leaves a readable spectrum, so
        // this row can never walk the layout into a single full-screen block.
        adjust: |c, d| {
            let dim = dim_for(c);
            let max_w = wrap_bar_width(dim, c.general.bar_spacing);
            let cur = crate::render::effective_bar_width(c, dim);
            c.general.bar_width = match d {
                Dir::Inc => (cur + 1).min(max_w),
                Dir::Dec => cur.saturating_sub(1).max(1),
            };
            // A wider bar may no longer leave room for the configured count.
            if c.general.bars > 0 {
                c.general.bars = c.general.bars.min(max_bars_that_fit(c)).max(1);
            }
        },
        reset: |c| c.general.bar_width = 2,
        set: Some(|c, v| {
            let dim = dim_for(c);
            let max_w = wrap_bar_width(dim, c.general.bar_spacing);
            c.general.bar_width = (v.round().max(1.0) as usize).min(max_w);
            if c.general.bars > 0 {
                c.general.bars = c.general.bars.min(max_bars_that_fit(c)).max(1);
            }
        }),
    },
    Row {
        label: |c| label("Bar Spacing", format!("{:>8}", c.general.bar_spacing)),
        adjust: |c, d| {
            c.general.bar_spacing = (c.general.bar_spacing as isize + d.step_usize(1))
                .clamp(0, 16) as usize
        },
        reset: |c| c.general.bar_spacing = 1,
        set: Some(|c, v| c.general.bar_spacing = (v.round().max(0.0) as usize).min(16)),
    },
    Row {
        label: |c| label("Framerate", format!("{:>8}", c.general.framerate)),
        adjust: |c, d| {
            c.general.framerate =
                (c.general.framerate as isize + d.step_usize(5)).clamp(10, 240) as u32
        },
        reset: |c| c.general.framerate = 60,
        set: Some(|c, v| c.general.framerate = (v.round() as i64).clamp(10, 240) as u32),
    },
    Row {
        label: |c| label("Orientation", format!("{:>8}", c.output.orientation.as_str())),
        adjust: |c, d| {
            const ORDER: [Orientation; 5] = [
                Orientation::Bottom,
                Orientation::Top,
                Orientation::Left,
                Orientation::Right,
                Orientation::Horizontal,
            ];
            c.output.orientation = cycle(&ORDER, &c.output.orientation, d);
        },
        reset: |c| c.output.orientation = Orientation::Bottom,
        set: None,
    },
    Row {
        label: |c| label("Output Method", format!("{:>8}", c.output.method.as_str())),
        adjust: |c, d| {
            let list = output_methods();
            c.output.method = cycle(&list, &c.output.method, d);
        },
        reset: |c| c.output.method = OutputMethod::Noncurses,
        set: None,
    },
    Row {
        label: |c| label("Input Method", format!("{:>8}", c.input.method.as_str())),
        adjust: |c, d| c.input.method = cycle(&INPUT_METHODS, &c.input.method, d),
        reset: |c| c.input.method = InputMethod::Pipewire,
        set: None,
    },
    Row {
        label: |c| {
            let name = format!("{:<20}", "Foreground");
            let value = mono(&c.color.foreground);
            // The value is drawn in its own vivid colour: the span's fg beats
            // the row highlight, and matches the preview/app so cycling this
            // row shows the true rendered colour, not the terminal's theme.
            Line::from(vec![
                Span::raw(name),
                Span::styled(
                    format!("{value:>10}"),
                    Style::default().fg(
                        c.color
                            .foreground
                            .to_rgb()
                            .map(|c| Color::Rgb(c.r, c.g, c.b))
                            .unwrap_or(Color::Reset),
                    ),
                ),
            ])
        },
        adjust: |c, d| c.color.foreground = cycle(&FG_COLORS, &c.color.foreground, d),
        reset: |c| c.color.foreground = TerminalColor::Default,
        set: None,
    },
    Row {
        label: |c| {
            let name = format!("{:<20}", "Background");
            let value = mono(&c.color.background);
            // A two-cell chip in the current background colour: the span's bg
            // beats the row highlight, so black/white/blue are visible while
            // selected. `default` renders as the terminal's own background.
            Line::from(vec![
                Span::raw(name),
                Span::raw(format!("{value:>8}")),
                Span::styled("  ", Style::default().bg(terminal_color(&c.color.background))),
            ])
        },
        adjust: |c, d| c.color.background = cycle(&BG_COLORS, &c.color.background, d),
        reset: |c| c.color.background = TerminalColor::Default,
        set: None,
    },
    Row {
        label: |c| label("Gradient", format!("{:>8}", onoff(c.color.gradient))),
        adjust: |c, _| c.color.gradient = !c.color.gradient,
        reset: |c| c.color.gradient = false,
        set: None,
    },
    Row {
        label: |c| {
            // Name plus a live swatch of the current gradient colors, so the
            // cycle is visible even while Gradient is OFF.
            let mut spans = vec![Span::raw(format!(
                "{:<20}{:>10} ",
                "Gradient Palette",
                GRADIENTS[current_gradient_idx(c)].0
            ))];
            for col in c.color.gradient_colors.iter().take(8) {
                if let Some(rgb) = col.to_rgb() {
                    spans.push(Span::styled("▌", Style::default().fg(Color::Rgb(rgb.r, rgb.g, rgb.b))));
                }
            }
            Line::from(spans)
        },
        adjust: |c, d| {
            let idx = current_gradient_idx(c);
            let step = match d {
                Dir::Inc => 1,
                Dir::Dec => GRADIENTS.len() - 1,
            };
            c.color.gradient_colors =
                palette_colors(&GRADIENTS[(idx + step) % GRADIENTS.len()].1);
        },
        reset: |c| c.color.gradient_colors = palette_colors(&GRADIENTS[0].1),
        set: None,
    },
    Row {
        label: |c| label("Draw Peaks", format!("{:>8}", onoff(c.output.draw_peaks))),
        adjust: |c, _| c.output.draw_peaks = !c.output.draw_peaks,
        reset: |c| c.output.draw_peaks = true,
        set: None,
    },
    Row {
        label: |c| label("Idle Bar Heads", format!("{:>8}", onoff(c.output.show_idle_bar_heads))),
        adjust: |c, _| c.output.show_idle_bar_heads = !c.output.show_idle_bar_heads,
        reset: |c| c.output.show_idle_bar_heads = true,
        set: None,
    },
    Row {
        label: |c| label("Waveform", format!("{:>8}", onoff(c.output.waveform))),
        adjust: |c, _| c.output.waveform = !c.output.waveform,
        reset: |c| c.output.waveform = false,
        set: None,
    },
    Row {
        // Ceiling 10: the smoothing arms only reach a few bins out at that
        // setting; values past it go inert and make the row look dead.
        label: |c| label("Monstercat", format!("{:>8.1}", c.smoothing.monstercat)),
        adjust: |c, d| {
            c.smoothing.monstercat = (c.smoothing.monstercat + d.step(0.5)).clamp(0.0, 10.0)
        },
        reset: |c| c.smoothing.monstercat = 0.0,
        set: Some(|c, v| c.smoothing.monstercat = v.clamp(0.0, 10.0)),
    },
    Row {
        label: |c| {
            label(
                "Noise Red.",
                format!("{:>7}%", (c.smoothing.noise_reduction * 100.0).round() as i64),
            )
        },
        adjust: |c, d| {
            let pct = (c.smoothing.noise_reduction * 100.0).round() as i64;
            let pct = match d {
                Dir::Inc => (pct + 5).min(100),
                Dir::Dec => (pct - 5).max(0),
            };
            c.smoothing.noise_reduction = pct as f64 / 100.0;
        },
        reset: |c| c.smoothing.noise_reduction = 0.77,
        // The row displays a percentage, so a typed value is a percentage too.
        set: Some(|c, v| c.smoothing.noise_reduction = v.clamp(0.0, 100.0) / 100.0),
    },
    Row {
        label: |c| label("Gravity", format!("{:>8.0}", c.smoothing.gravity)),
        adjust: |c, d| {
            c.smoothing.gravity = (c.smoothing.gravity + d.step(5.0)).clamp(0.0, 1000.0)
        },
        reset: |c| c.smoothing.gravity = 100.0,
        set: Some(|c, v| c.smoothing.gravity = v.clamp(0.0, 1000.0)),
    },
    Row {
        label: |c| label("Wrap Bars", format!("{:>8}", onoff(c.general.wrap_bars))),
        adjust: |c, _| c.general.wrap_bars = !c.general.wrap_bars,
        reset: |c| c.general.wrap_bars = false,
        set: None,
    },
    Row {
        label: |c| {
            label(
                "Waveform Style",
                match c.output.waveform_style {
                    crate::config::WaveformStyle::Line => "line".to_string(),
                    crate::config::WaveformStyle::Area => "area".to_string(),
                },
            )
        },
        adjust: |c, d| {
            c.output.waveform_style = match (c.output.waveform_style, d) {
                (crate::config::WaveformStyle::Line, Dir::Inc)
                | (crate::config::WaveformStyle::Area, Dir::Dec) => {
                    crate::config::WaveformStyle::Area
                }
                _ => crate::config::WaveformStyle::Line,
            }
        },
        reset: |c| c.output.waveform_style = crate::config::WaveformStyle::Line,
        set: None,
    },
    Row {
        label: |c| label("Waveform Baseline", format!("{:>8}", onoff(c.output.waveform_baseline))),
        adjust: |c, _| c.output.waveform_baseline = !c.output.waveform_baseline,
        reset: |c| c.output.waveform_baseline = true,
        set: None,
    },
    Row {
        label: |c| label("Waveform Dynamics", format!("{:>8}", onoff(c.output.waveform_dynamics))),
        adjust: |c, _| c.output.waveform_dynamics = !c.output.waveform_dynamics,
        reset: |c| c.output.waveform_dynamics = false,
        set: None,
    },
    Row {
        label: |c| label("Waveform Graticule", format!("{:>8}", onoff(c.output.waveform_graticule))),
        adjust: |c, _| c.output.waveform_graticule = !c.output.waveform_graticule,
        reset: |c| c.output.waveform_graticule = false,
        set: None,
    },
    Row {
        label: |c| label("Bar Reflection", format!("{:>8}", onoff(c.output.bar_reflection))),
        adjust: |c, _| c.output.bar_reflection = !c.output.bar_reflection,
        reset: |c| c.output.bar_reflection = false,
        set: None,
    },
    Row {
        label: |c| label("Stereo Divider", format!("{:>8}", onoff(c.output.stereo_divider))),
        adjust: |c, _| c.output.stereo_divider = !c.output.stereo_divider,
        reset: |c| c.output.stereo_divider = false,
        set: None,
    },
    Row {
        label: |c| label("Baseline Ruler", format!("{:>8}", onoff(c.output.baseline_ruler))),
        adjust: |c, _| c.output.baseline_ruler = !c.output.baseline_ruler,
        reset: |c| c.output.baseline_ruler = false,
        set: None,
    },
    Row {
        label: |c| label("Bar Cap", format!("{:>8}", onoff(c.output.bar_cap))),
        adjust: |c, _| c.output.bar_cap = !c.output.bar_cap,
        reset: |c| c.output.bar_cap = false,
        set: None,
    },
    Row {
        label: |c| label("Bright Peaks", format!("{:>8}", onoff(c.output.bright_peaks))),
        adjust: |c, _| c.output.bright_peaks = !c.output.bright_peaks,
        reset: |c| c.output.bright_peaks = true,
        set: None,
    },
    Row {
        label: |c| label("Beat Pulse", format!("{:>8}", onoff(c.output.beat_pulse))),
        adjust: |c, _| c.output.beat_pulse = !c.output.beat_pulse,
        reset: |c| c.output.beat_pulse = false,
        set: None,
    },
    Row {
        label: |c| label("ASCII Glyphs", format!("{:>8}", onoff(c.output.ascii_glyphs))),
        adjust: |c, _| c.output.ascii_glyphs = !c.output.ascii_glyphs,
        reset: |c| c.output.ascii_glyphs = false,
        set: None,
    },
    Row {
        label: |c| label("Reduce Motion", format!("{:>8}", onoff(c.output.reduce_motion))),
        adjust: |c, _| c.output.reduce_motion = !c.output.reduce_motion,
        reset: |c| c.output.reduce_motion = false,
        set: None,
    },
    Row {
        label: |c| label("Spectrum Mode", format!("{:>8}", onoff(c.color.spectrum))),
        adjust: |c, _| c.color.spectrum = !c.color.spectrum,
        reset: |c| c.color.spectrum = false,
        set: None,
    },
    Row {
        label: |c| label("Background Gradient", format!("{:>8}", onoff(c.color.background_gradient))),
        adjust: |c, _| c.color.background_gradient = !c.color.background_gradient,
        reset: |c| c.color.background_gradient = false,
        set: None,
    },
];

/// Minimum terminal width that shows the live preview pane. Narrower terms
/// keep the menu as a single column so it stays usable.
const PREVIEW_MIN_WIDTH: u16 = 80;

/// Column (relative to the list's left edge) where a row's value starts.
/// The item text is indented by the left border (1) plus the `>> ` highlight
/// symbol (3), and the label is left-padded to 20 columns, so the value column
/// begins 24 cells in. Clicking at or past it nudges the row instead of only
/// selecting it.
const VALUE_COLUMN: u16 = 24;

/// Append a character to a numeric entry buffer. Accepts ASCII digits and at
/// most one decimal point; returns whether the buffer changed.
fn push_typed(buf: &mut String, c: char) -> bool {
    let ok = c.is_ascii_digit() || (c == '.' && !buf.contains('.'));
    if ok {
        buf.push(c);
    }
    ok
}

/// Parse a typed entry buffer into a value. `None` for an empty or malformed
/// buffer (e.g. a lone ".").
fn parse_typed(buf: &str) -> Option<f64> {
    if buf.is_empty() {
        None
    } else {
        buf.parse::<f64>().ok()
    }
}

/// Map a mouse click position onto a list row index, given where the list
/// widget was drawn last frame and its current scroll `offset` (the index of
/// the topmost visible row).
///
/// Crossterm mouse coordinates are 0-based (the SGR parser subtracts one), so
/// the first content row sits at `area.y + 1` (just inside the top border) and
/// visible row `v` maps to absolute list row `offset + v`. `None` when the
/// click landed outside the list's content area — the top/bottom borders, the
/// trailing preview pane, or the margins.
fn click_to_index(list_area: &Rect, offset: usize, column: u16, row: u16) -> Option<usize> {
    if column < list_area.x
        || column >= list_area.right()
        || row <= list_area.y
        || row + 1 >= list_area.bottom()
    {
        return None;
    }
    let visible = row as i32 - (list_area.y as i32 + 1);
    if visible < 0 {
        None
    } else {
        Some(offset + visible as usize)
    }
}

pub fn run_menu(config: &mut Config) -> Result<(), std::io::Error> {
    // `TerminalGuard::suspend()` turned raw mode off before handing the terminal
    // over, so the menu owns raw mode for the duration of its lifetime.
    enable_raw_mode()?;
    crossterm::execute!(std::io::stdout(), EnableMouseCapture)?;

    // Terminal::new()/Terminal::clear() snapshot the cursor position by querying
    // the terminal (ESC[6n), which hangs or fails when nothing answers the
    // query -- a pipe-backed stdout or a headless PTY, for instance. Fullscreen
    // rendering never needs the cursor's position, so build the terminal with an
    // explicit Fullscreen viewport and clear through the backend's region
    // primitive instead of the cursor-aware Terminal::clear().
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
        )?;
    execute!(std::io::stdout(), Clear(ClearType::All))?;

    // Snapshot the config so "Discard && Exit" can restore both memory and file.
    let snapshot = config.clone();

    let mut list_state = ListState::default();
    list_state.select(Some(0));

    // Where the list widget was drawn last frame — used to hit-test mouse
    // clicks into a row selection.
    let mut list_area = Rect::default();

    // In-progress direct numeric entry (typing on a numeric row). While this is
    // `Some`, keystrokes edit the buffer instead of navigating; Enter applies
    // it through the row's `set`, Esc cancels.
    let mut edit: Option<String> = None;

    // Animated synthetic spectrum for the live preview pane. Only advanced on
    // redraws, so it stays in lockstep with the menu's 100 ms poll.
    let mut preview = Preview::new();

    const NUM_ITEMS: usize = ROWS.len() + 2;
    const SAVE_EXIT: usize = NUM_ITEMS - 2;     // "Save && Exit"
    const DISCARD_EXIT: usize = NUM_ITEMS - 1;  // "Discard && Exit"

    // Restore defaults on demand (`r` resets the selected row). Everything
    // else in the loop saves as it changes.
    let reset_selected = |config: &mut Config, list_state: &mut ListState| {
        let Some(i) = list_state.selected() else { return };
        if i < ROWS.len() {
            (ROWS[i].reset)(config);
            config.save().ok();
        }
    };

    loop {
        preview.advance();

        let mut items: Vec<Line> = ROWS.iter().map(|r| (r.label)(config)).collect();
        items.push(Line::from("Save && Exit".to_string()));
        items.push(Line::from("Discard && Exit".to_string()));

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints([
                    Constraint::Length(5),
                    Constraint::Min(0),
                ].as_ref())
                .split(f.area());

            let title_text = vec![
                Line::from(Span::styled(
                    " Zava Configuration Menu ",
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
                Line::from(""),
                if let Some(buf) = &edit {
                    Line::from(format!(
                        "  editing: {}_   Enter = apply   Esc = cancel   Backspace = delete",
                        buf
                    ))
                } else {
                    Line::from("  \u{2191}\u{2193} nav  \u{2190}\u{2192} adjust  r reset  Enter action  Esc close  mouse/type = value")
                },
            ];
            let title = Paragraph::new(title_text)
                .block(Block::default().title(" Zava ").borders(Borders::ALL).border_style(Style::default().fg(Color::Cyan)));
            f.render_widget(title, chunks[0]);

            // Split the body into the settings list and (on wide terminals) the
            // live preview pane.
            let body = chunks[1];
            let (list_rect, preview_rect) = if body.width >= PREVIEW_MIN_WIDTH {
                let parts = Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([
                        Constraint::Min(0),
                        Constraint::Length(36),
                    ].as_ref())
                    .split(body);
                (parts[0], parts[1])
            } else {
                (body, Rect::default())
            };
            list_area = list_rect;

            let list_items: Vec<ListItem> = items
                .into_iter()
                .map(ListItem::new)
                .collect();

let list = List::new(list_items)
                .block(Block::default().title(" Settings ").borders(Borders::ALL).border_style(Style::default().fg(Color::Blue)))
                // Neutral highlight: a light backdrop with no fg override, so
                // self-coloured spans (fg/bg chips, palette swatch) stay
                // readable while the row is selected — a solid blue background
                // used to drown the very colours the palette rows are showing.
                .highlight_style(Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD))
                .highlight_symbol(">> ");
            f.render_stateful_widget(list, list_rect, &mut list_state);

            if preview_rect.width > 0 && preview_rect.height > 2 {
                let pw = (preview_rect.width - 2) as usize;
                let ph = (preview_rect.height - 2) as usize;
                let grid = render_preview(config, &preview, pw, ph);
                let lines = to_lines(&grid);
                let pane = Paragraph::new(lines)
                    .block(Block::default().title(" Preview ").borders(Borders::ALL).border_style(Style::default().fg(Color::Magenta)));
                f.render_widget(pane, preview_rect);
            }
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    // Ctrl+C must be handled explicitly: raw mode disables ISIG, so
                    // the terminal never raises SIGINT while the menu owns input.
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
                    {
                        break;
                    }

                    // While a direct entry is in progress every key edits the
                    // buffer: Enter applies, Esc cancels, Backspace deletes.
                    if edit.is_some() {
                        match key.code {
                            KeyCode::Enter => {
                                if let (Some(i), Some(buf)) = (list_state.selected(), edit.take())
                                {
                                    if let (Some(set), Some(v)) =
                                        (ROWS.get(i).and_then(|r| r.set), parse_typed(&buf))
                                    {
                                        set(config, v);
                                        config.save().ok();
                                    }
                                }
                            }
                            KeyCode::Esc => edit = None,
                            KeyCode::Backspace => {
                                if let Some(buf) = edit.as_mut() {
                                    buf.pop();
                                }
                            }
                            KeyCode::Char(c) => {
                                if let Some(buf) = edit.as_mut() {
                                    push_typed(buf, c);
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    match key.code {
                        KeyCode::Esc | KeyCode::Char('m') | KeyCode::Char('q') => break,
                        KeyCode::Up => {
                            let cur = list_state.selected().unwrap_or(0);
                            list_state.select(Some(cur.saturating_sub(1)));
                        }
                        KeyCode::Down => {
                            let cur = list_state.selected().unwrap_or(0);
                            list_state.select(Some((cur + 1).min(NUM_ITEMS - 1)));
                        }
                        KeyCode::Home => list_state.select(Some(0)),
                        KeyCode::End | KeyCode::PageDown => {
                            list_state.select(Some(NUM_ITEMS - 1))
                        }
                        KeyCode::PageUp => list_state.select(Some(0)),
                        KeyCode::Left => self::adjust(config, &mut list_state, Dir::Dec),
                        KeyCode::Right => self::adjust(config, &mut list_state, Dir::Inc),
                        KeyCode::Char('r') | KeyCode::Char('R') => {
                            reset_selected(config, &mut list_state)
                        }
                        // Typing a digit (or ".") on a numeric row starts a
                        // direct value entry; non-numeric rows ignore it.
                        KeyCode::Char(c)
                            if c.is_ascii_digit() || c == '.' => {
                            if let Some(i) = list_state.selected() {
                                if i < ROWS.len() && ROWS[i].set.is_some() {
                                    let mut buf = String::new();
                                    push_typed(&mut buf, c);
                                    edit = Some(buf);
                                }
                            }
                        }
                        KeyCode::Enter => {
                            match list_state.selected() {
                                Some(i) if i == SAVE_EXIT => break,
                                Some(i) if i == DISCARD_EXIT => {
                                    *config = snapshot.clone();
                                    config.save().ok();
                                }
                                _ => {}
                            }
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => {
                    match mouse.kind {
                        MouseEventKind::ScrollUp => {
                            let cur = list_state.selected().unwrap_or(0);
                            list_state.select(Some(cur.saturating_sub(1)));
                        }
                        MouseEventKind::ScrollDown => {
                            let cur = list_state.selected().unwrap_or(0);
                            list_state.select(Some((cur + 1).min(NUM_ITEMS - 1)));
                        }
                        MouseEventKind::Down(MouseButton::Left) => {
                            let offset = list_state.offset();
                            if let Some(idx) =
                                click_to_index(&list_area, offset, mouse.column, mouse.row)
                            {
                                // Blank space below the last item maps past the
                                // item count; clamping it to the last row would
                                // silently run "Discard && Exit".
                                if idx < NUM_ITEMS {
                                    list_state.select(Some(idx));
                                    // Clicking a row selects and activates it:
                                    // the action rows run, and a click in a
                                    // row's value column nudges the value by
                                    // one step.
                                    match idx {
                                        SAVE_EXIT => break,
                                        DISCARD_EXIT => {
                                            *config = snapshot.clone();
                                            config.save().ok();
                                        }
                                        _ if mouse.column >= list_area.x + VALUE_COLUMN => {
                                            self::adjust(config, &mut list_state, Dir::Inc);
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }

    execute!(std::io::stdout(), DisableMouseCapture)?;
    execute!(std::io::stdout(), Clear(ClearType::All))?;
    // Hand raw mode back so `TerminalGuard::resume()` can re-enable it cleanly.
    disable_raw_mode()?;
    Ok(())
}

/// Apply the selected row's adjustment and persist it, except on the fixed
/// "Save && Exit" / "Discard && Exit" rows.
fn adjust(config: &mut Config, list_state: &mut ListState, dir: Dir) {
    let Some(i) = list_state.selected() else { return };
    if i >= SAVE_ROWS_START {
        return;
    }
    (ROWS[i].adjust)(config, dir);
    config.save().ok();
}

const SAVE_ROWS_START: usize = ROWS.len();

#[cfg(test)]
mod tests {
    use super::*;

    /// `r` on a row restores that row's own default(s), not a blanket reset.
    #[test]
    fn reset_restores_the_selected_rows_default() {
        let mut config = Config::default();
        config.general.sensitivity = 230.0;
        (ROWS[0].reset)(&mut config);
        assert_eq!(config.general.sensitivity, 100.0);

        config.smoothing.monstercat = 9.9;
        (ROWS[16].reset)(&mut config);
        assert_eq!(config.smoothing.monstercat, 0.0);

        config.general.bars = 24;
        config.general.bar_width = 8;
        config.general.bar_spacing = 3;
        (ROWS[2].reset)(&mut config);
        assert_eq!(config.general.bars, 0);
        assert_eq!(config.general.bar_width, 2);
        assert_eq!(config.general.bar_spacing, 1);

        config.color.gradient_colors = palette_colors(&GRADIENTS[2].1);
        (ROWS[12].reset)(&mut config);
        assert_eq!(config.color.gradient_colors, palette_colors(&GRADIENTS[0].1));
    }

    /// Every row's reset strips the *configurable* state, so a reset can never
    /// write a non-default value back into the row it was called on.
    #[test]
    fn reset_rows_cover_the_whole_table() {
        let defaults = Config::default();
        let mut config = defaults.clone();
        for row in ROWS {
            (row.adjust)(&mut config, Dir::Inc);
            (row.reset)(&mut config);
        }
        // A full sweep with adjustments followed by resets must not panic and
        // must leave the general/output defaults intact.
        assert_eq!(config.general.sensitivity, defaults.general.sensitivity);
        assert_eq!(config.output.orientation, defaults.output.orientation);
    }

    /// Crossterm mouse coordinates are 0-based; visible row `v` maps to list
    /// row `offset + v`. Clicks outside the list content (on the borders, the
    /// right preview pane, or the margins) must be ignored.
    #[test]
    fn click_positions_map_to_list_rows() {
        let area = Rect::new(2, 7, 72, 21);
        // First content row sits just inside the top border at row 7 + 1 = 8.
        assert_eq!(click_to_index(&area, 0, 3, 8), Some(0));
        // Row 10 (Background) sits at row 18.
        assert_eq!(click_to_index(&area, 0, 5, 18), Some(10));
        // A click on the right pane (outside the list width) is ignored.
        assert_eq!(click_to_index(&area, 0, 100, 18), None);
        // Clicks on the top or bottom border are ignored.
        assert_eq!(click_to_index(&area, 0, 3, 7), None);
        assert_eq!(click_to_index(&area, 0, 3, 27), None);
        // A click past the bottom border is ignored too.
        assert_eq!(click_to_index(&area, 0, 3, 100), None);
    }

    /// A scrolled list must map clicks by adding the scroll offset, otherwise
    /// the first visible row always selects row 0.
    #[test]
    fn click_to_index_respects_scroll_offset() {
        let area = Rect::new(2, 7, 72, 21);
        // Topmost visible row is row 5; a click on it selects row 5, not 0.
        assert_eq!(click_to_index(&area, 5, 3, 8), Some(5));
        assert_eq!(click_to_index(&area, 5, 3, 10), Some(7));
    }

    /// Typed entry accepts digits and a single decimal point, rejecting a
    /// second point and any other character.
    #[test]
    fn typed_entry_accepts_digits_and_one_dot() {
        let mut buf = String::new();
        assert!(push_typed(&mut buf, '1'));
        assert!(push_typed(&mut buf, '2'));
        assert!(push_typed(&mut buf, '.'));
        assert!(push_typed(&mut buf, '5'));
        assert!(!push_typed(&mut buf, '.'), "a second dot must be rejected");
        assert!(!push_typed(&mut buf, 'x'), "letters must be rejected");
        assert_eq!(buf, "12.5");
        assert_eq!(parse_typed(&buf), Some(12.5));
        assert_eq!(parse_typed(""), None);
        assert_eq!(parse_typed("."), None);
    }

    /// Every `set` clamp mirrors its row's `adjust` clamp, so a typed value
    /// can never write a value the arrows could not reach.
    #[test]
    fn set_rows_clamp_typed_values() {
        let mut config = Config::default();

        (ROWS[0].set.unwrap())(&mut config, -50.0);
        assert_eq!(config.general.sensitivity, 1.0);
        (ROWS[0].set.unwrap())(&mut config, 9999.0);
        assert_eq!(config.general.sensitivity, 500.0);

        (ROWS[4].set.unwrap())(&mut config, 99.0);
        assert_eq!(config.general.bar_spacing, 16);

        (ROWS[5].set.unwrap())(&mut config, 1.0);
        assert_eq!(config.general.framerate, 10);
        (ROWS[5].set.unwrap())(&mut config, 9999.0);
        assert_eq!(config.general.framerate, 240);

        (ROWS[16].set.unwrap())(&mut config, 99.0);
        assert_eq!(config.smoothing.monstercat, 10.0);

        // Noise Red. is typed as a percentage.
        (ROWS[17].set.unwrap())(&mut config, 42.0);
        assert!((config.smoothing.noise_reduction - 0.42).abs() < 1e-9);

        (ROWS[18].set.unwrap())(&mut config, 9999.0);
        assert_eq!(config.smoothing.gravity, 1000.0);

        // A non-numeric row is not type-editable.
        assert!(ROWS[1].set.is_none());
        assert!(ROWS[15].set.is_none());
    }

    /// Typing 0 into Bars means "auto", matching the row's label.
    #[test]
    fn typed_zero_means_auto_bars() {
        let mut config = Config::default();
        config.general.bars = 40;
        (ROWS[2].set.unwrap())(&mut config, 0.0);
        assert_eq!(config.general.bars, 0);
    }
}
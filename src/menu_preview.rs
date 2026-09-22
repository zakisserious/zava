use crate::config::{Config, Orientation};
use crate::render::color::ColorMap;
use crate::render::terminal::{
    waveform_cells, mirrored_glyph, mirrored_level, ASCII_CHARS, BarLayout, HORIZ_ASCII_CHARS,
    HORIZ_LEFT_CHARS, TOP_CHARS, VERT_CHARS,
};
use crate::render::effective_bar_width;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Synthetic level feed for the menu's live preview pane.
///
/// The menu redraws on a 100 ms poll; each poll `advance()` breathes a
/// pseudo-spectrum once and decays/updates the per-bar peak trail exactly
/// like the real renderer does, so the preview feels alive without touching
/// the audio backend.
pub(crate) struct Preview {
    bars: Vec<f64>,
    peaks: Vec<f64>,
    phase: f64,
}

impl Preview {
    pub(crate) fn new() -> Self {
        Self {
            bars: vec![0.0; 64],
            peaks: vec![0.0; 64],
            phase: 0.0,
        }
    }

    pub(crate) fn advance(&mut self) -> usize {
        self.phase += 0.10;
        let n = self.bars.len();
        for i in 0..n {
            let t = i as f64 / (n - 1) as f64;
            // A lump near the low end that rolls off gently on both sides and
            // breathes — reads as "music" a human recognises, unlike a wall of
            // identical bars.
            let env = (1.15 - (t - 0.33).abs() * 2.6).clamp(0.0, 1.0);
            let breathe = 0.7 + 0.3 * (self.phase * 0.8).sin();
            let ripple = 0.10 * (self.phase * 2.3 + t * 11.0).sin();
            let v = (env * breathe + ripple * env).clamp(0.0, 1.0);
            self.bars[i] = v;
            // Peak hold/decay, mirroring the terminal renderer's rules.
            if v > self.peaks[i] {
                self.peaks[i] = v;
            } else {
                let decay = 0.02;
                let next = self.peaks[i] - decay;
                self.peaks[i] = if next < decay { v } else { next.max(v) };
            }
        }
        n
    }

    pub(crate) fn level(&self, i: usize) -> f64 {
        self.bars.get(i).copied().unwrap_or(0.0)
    }

    pub(crate) fn peak(&self, i: usize) -> f64 {
        self.peaks.get(i).copied().unwrap_or(0.0)
    }

    /// Instantaneous waveform trace for oscilloscope mode.
    fn trace(&self, width: usize) -> Vec<f64> {
        let phase = self.phase;
        (0..width)
            .map(|x| {
                let t = if width > 1 {
                    x as f64 / (width - 1) as f64
                } else {
                    0.0
                };
                // A travelling sine, gentle enough that one cycle stays
                // readable across the preview pane (the old trace stuffed ~8
                // cycles of full-height amplitude in, which the sub-row band
                // renderer turned into a thick jittery slab).
                0.5 + 0.34 * (t * 5.5 + phase * 1.1).sin()
            })
            .collect()
    }
}

/// One rendered cell of the preview grid.
#[derive(Clone, Copy)]
pub(crate) struct Cell {
    pub ch: char,
    /// Ink colour; `None` = terminal default (used for Default fg/black).
    pub color: Option<(u8, u8, u8)>,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            color: None,
        }
    }
}

/// Render the current config into a `w` x `h` cell grid for the menu pane.
///
/// Ports the terminal renderer's per-orientation math onto a grid instead of
/// an ANSI stream, using the same glyph tables, layout dealout and peak
/// rules, so the preview is a faithful picture of the live visualization.
pub(crate) fn render_preview(
    config: &Config,
    preview: &Preview,
    w: usize,
    h: usize,
) -> Vec<Vec<Cell>> {
    let mut grid = vec![vec![Cell::default(); w]; h];
    if w == 0 || h == 0 {
        return grid;
    }

    let fg = config.color.foreground.to_rgb().map(|c| (c.r, c.g, c.b));
    let idle = config.output.show_idle_bar_heads;
    let peaks_on = config.output.draw_peaks;
    let spacing = config.general.bar_spacing;
    let ascii = config.output.ascii_glyphs;
    let vert = if ascii { ASCII_CHARS } else { VERT_CHARS };
    let top = if ascii { ASCII_CHARS } else { TOP_CHARS };
    let horiz = if ascii { HORIZ_ASCII_CHARS } else { HORIZ_LEFT_CHARS };

    if config.output.waveform {
        let half_max = h * 2 - 1;
        let spans = waveform_cells(&preview.trace(w), w, half_max);
        let fill_area = config.output.waveform_style == crate::config::WaveformStyle::Area;
        let fill_ch = if ascii { '#' } else { '\u{2588}' };
        for y in 0..h {
            let upper_row = y * 2;
            let lower_row = upper_row + 1;
            for (x, &(lo, hi)) in spans.iter().enumerate() {
                let upper = lo <= upper_row && upper_row <= hi;
                let lower = lo <= lower_row && lower_row <= hi;
                let ch = if upper && lower {
                    '\u{2588}'
                } else if upper {
                    '\u{2580}'
                } else if lower {
                    '\u{2584}'
                } else if fill_area && lower_row > lo {
                    // Cell sits below the trace band: solid area fill.
                    fill_ch
                } else {
                    ' '
                };
                if ch != ' ' {
                    grid[y][x] = Cell { ch, color: fg };
                }
            }
        }
        return grid;
    }

    let vertical = matches!(
        config.output.orientation,
        Orientation::Left | Orientation::Right
    );
    let axis = if vertical { h } else { w };

    let fit = crate::render::bars_that_fit(
        axis,
        effective_bar_width(config, axis),
        spacing,
    );
    let fit_even = if config.output.channels == 2 { fit - fit % 2 } else { fit };
    let n = if config.general.bars == 0 {
        fit_even
    } else {
        config.general.bars.min(fit_even.max(1)).max(1)
    };
    let n = n.min(preview_level_cap()); // layout still clamps to the axis

    let bar_width = effective_bar_width(config, axis).min(axis).max(1);
    let layout = BarLayout::new(axis, n, bar_width, spacing);
    let cmap = ColorMap::new(&config.color, h.max(1), n.max(1));

    let ink = |y: usize, x: usize| -> Option<(u8, u8, u8)> {
        cmap.color_at(y, x)
            .map(|c| (c.r, c.g, c.b))
            .or(fg)
    };

    let mut cursor = 0usize; // next free axis cell, walks with the bars' gaps
    for b_idx in 0..n {
        cursor += layout.gap_before(b_idx);
        let bar = preview.level(b_idx);
        let peak = preview.peak(b_idx);
        match config.output.orientation {
            Orientation::Bottom => {
                let max_sub = (h as f64 * 8.0 * config.general.max_height).max(1.0);
                let sub_h = bar * max_sub;
                let peak_sub = peak * max_sub;
                for c in 0..bar_width {
                    let x = cursor + c;
                    if x >= w {
                        break;
                    }
                    for y in (0..h).rev() {
                        let cell_val = sub_h - (y as f64 * 8.0);
                        let mut glyph = if cell_val >= 8.0 {
                            vert[7]
                        } else if cell_val >= 1.0 {
                            vert[(cell_val as usize).clamp(1, 8) - 1]
                        } else if y == 0 && idle {
                            vert[0]
                        } else {
                            ' '
                        };
                        if glyph == ' ' && peaks_on {
                            let peak_cell = peak_sub - (y as f64 * 8.0);
                            if peak_cell >= 0.0 && peak_cell < 8.0 {
                                glyph = '_';
                            }
                        }
                        if glyph != ' ' {
                            grid[y][x] = Cell {
                                ch: glyph,
                                color: ink(y, b_idx),
                            };
                        }
                    }
                }
            }
            Orientation::Top => {
                let max_sub = (h as f64 * 8.0 * config.general.max_height).max(1.0);
                for c in 0..bar_width {
                    let x = cursor + c;
                    if x >= w {
                        break;
                    }
                    for y in 0..h {
                        let level = ((bar * max_sub - (y as f64 * 8.0)).max(0.0) as usize).min(8);
                        let mut glyph = if level >= 8 {
                            top[7]
                        } else if level >= 1 {
                            top[level - 1]
                        } else if y == 0 && idle {
                            top[0]
                        } else {
                            ' '
                        };
                        if glyph == ' ' && peaks_on {
                            let peak_sub = peak * max_sub - (y as f64 * 8.0);
                            if peak_sub >= 0.0 {
                                glyph = '▾';
                            }
                        }
                        if glyph != ' ' {
                            grid[y][x] = Cell {
                                ch: glyph,
                                color: ink(h - 1 - y, b_idx),
                            };
                        }
                    }
                }
            }
            Orientation::Left => {
                let max_sub = (w as f64 * 8.0 * config.general.max_height).max(1.0);
                for r in 0..bar_width {
                    let y = cursor + r;
                    if y >= h {
                        break;
                    }
                    for x in 0..w {
                        let cell_val = (bar * max_sub) - (x as f64 * 8.0);
                        let scale_level = (cell_val.max(0.0) as usize).min(8);
                        let mut glyph = if scale_level >= 8 {
                            horiz[7]
                        } else if scale_level >= 1 {
                            horiz[scale_level - 1]
                        } else if x == 0 && idle {
                            horiz[0]
                        } else {
                            ' '
                        };
                        if glyph == ' ' && peaks_on {
                            let peak_val = peak * max_sub - (x as f64 * 8.0);
                            if peak_val >= 0.0 && peak_val < 8.0 {
                                glyph = '|';
                            }
                        }
                        if glyph != ' ' {
                            grid[y][x] = Cell {
                                ch: glyph,
                                color: ink(y, b_idx),
                            };
                        }
                    }
                }
            }
            Orientation::Right => {
                let max_sub = (w as f64 * 8.0 * config.general.max_height).max(1.0);
                for r in 0..bar_width {
                    let y = cursor + r;
                    if y >= h {
                        break;
                    }
                    for x in 0..w {
                        let dist = w - 1 - x;
                        let cell_val = (bar * max_sub) - (dist as f64 * 8.0);
                        let scale_level = (cell_val.max(0.0) as usize).min(8);
                        let mut glyph = if scale_level >= 8 {
                            horiz[7]
                        } else if scale_level >= 1 {
                            horiz[scale_level - 1]
                        } else if dist == 0 && idle {
                            horiz[0]
                        } else {
                            ' '
                        };
                        if glyph == ' ' && peaks_on {
                            let peak_val = peak * max_sub - (dist as f64 * 8.0);
                            if peak_val >= 0.0 && peak_val < 8.0 {
                                glyph = '|';
                            }
                        }
                        if glyph != ' ' {
                            grid[y][x] = Cell {
                                ch: glyph,
                                color: ink(y, b_idx),
                            };
                        }
                    }
                }
            }
            Orientation::Horizontal => {
                let half = (h / 2).max(1);
                let half_max = (half as f64 * 8.0 * config.general.max_height).max(1.0);
                let upper_base = half;
                let lower_base = half - 1;
                for c in 0..bar_width {
                    let x = cursor + c;
                    if x >= w {
                        break;
                    }
                    for y in (0..h).rev() {
                        let (offset, is_upper) = if y >= upper_base {
                            (y - upper_base, true)
                        } else {
                            (lower_base - y, false)
                        };
                        let level = mirrored_level(bar * half_max, offset);
                        let idle_here = offset == 0 && idle;
                        let mut glyph = mirrored_glyph(is_upper, level, idle_here, config.output.ascii_glyphs);
                        if glyph == ' ' && peaks_on {
                            let residual = peak * half_max - offset as f64 * 8.0;
                            if (0.0..8.0).contains(&residual) {
                                glyph = '_';
                            }
                        }
                        if glyph != ' ' {
                            let colour_row = if is_upper {
                                (2 * half - 1).saturating_sub(y).min(h - 1)
                            } else {
                                y
                            };
                            grid[y][x] = Cell {
                                ch: glyph,
                                color: ink(colour_row, b_idx),
                            };
                        }
                    }
                }
            }
        }
        cursor += bar_width;
    }
    grid
}

fn preview_level_cap() -> usize {
    64
}

/// Turn a preview grid into ratatui lines, coalescing runs of the same colour
/// into single spans so the pane stays cheap to draw every 100 ms.
pub(crate) fn to_lines(grid: &[Vec<Cell>]) -> Vec<Line<'static>> {
    grid.iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut run = String::new();
            let mut run_color: Option<(u8, u8, u8)> = None;
            for cell in row {
                if cell.color != run_color {
                    if !run.is_empty() {
                        flush(&mut spans, &mut run, run_color);
                    }
                    run_color = cell.color;
                }
                run.push(cell.ch);
            }
            flush(&mut spans, &mut run, run_color);
            Line::from(spans)
        })
        .collect()
}

fn flush(spans: &mut Vec<Span<'static>>, run: &mut String, color: Option<(u8, u8, u8)>) {
    if run.is_empty() {
        return;
    }
    let span = match color {
        Some((r, g, b)) => Span::styled(std::mem::take(run), Style::default().fg(Color::Rgb(r, g, b))),
        None => Span::raw(std::mem::take(run)),
    };
    spans.push(span);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn preview_bars_stay_in_unit_range_as_they_advance() {
        let mut p = Preview::new();
        for _ in 0..30 {
            let n = p.advance();
            for i in 0..n {
                let v = p.level(i);
                assert!((0.0..=1.0).contains(&v), "level {i} out of range: {v}");
                assert!(p.peak(i) >= v, "peak fell below its bar at {i}");
            }
        }
    }

    #[test]
    fn preview_grid_matches_its_dimensions() {
        let config = Config::default();
        let p = Preview::new();
        for (w, h) in [(30usize, 18usize), (34, 20), (10, 4)] {
            let grid = render_preview(&config, &p, w, h);
            assert_eq!(grid.len(), h);
            assert!(grid.iter().all(|row| row.len() == w));
            assert!(grid.iter().flatten().all(|c| c.ch != '\0'));
        }
    }

    #[test]
    fn fixed_bar_count_is_reflected_in_the_grid() {
        let mut config = Config::default();
        config.general.bars = 12;
        config.output.channels = 1;
        let p = Preview::new();
        // A vertical block must appear somewhere in the bottom row given
        // enough width for 12 bars.
        let grid = render_preview(&config, &p, 60, 16);
        let ink = grid[0].iter().filter(|c| c.ch != ' ').count();
        assert!(ink > 0, "fixed 12 bars produced no visible bottom-row ink");
    }

    /// The waveform trace must draw as a thin, readable sine that scrolls
    /// smoothly frame to frame — not a thick slab that flips between shapes
    /// every advance (the old full-height, ~8-cycle trace wobbled badly).
    #[test]
    fn waveform_trace_stays_thin_and_stable() {
        let mut p = Preview::new();
        let traces: Vec<Vec<f64>> = (0..12)
            .map(|_| {
                let t = p.trace(34);
                p.advance();
                t
            })
            .collect();
        // Each frame is unit-range.
        assert!(traces.iter().flatten().all(|&v| (0.0..=1.0).contains(&v)));
        // Band(column) wiggle between consecutive frames is small: the trace
        // moves gradually instead of jumping around.
        for pair in traces.windows(2) {
            let drift: f64 = pair[0]
                .iter()
                .zip(&pair[1])
                .map(|(a, b)| (a - b).abs())
                .sum();
            assert!(
                drift < 4.0,
                "trace jumped too far between frames: drift {drift}"
            );
        }
    }

    /// With vivid RGB the preview's flat (no-gradient) ink matches the app's
    /// renderer for the same configured colour.
    #[test]
    fn preview_uses_vivid_rgb_foreground() {
        let mut config = Config::default();
        config.color.foreground = crate::config::TerminalColor::Red;
        config.general.bars = 3;
        config.output.channels = 1;
        config.color.gradient = false;
        let p = Preview::new();
        let grid = render_preview(&config, &p, 40, 14);
        let colors: Vec<(u8, u8, u8)> = grid
            .iter()
            .flatten()
            .filter_map(|c| c.color)
            .collect();
        assert!(!colors.is_empty(), "no inked cells in the preview");
        assert!(
            colors.iter().all(|&c| c == (255, 0, 0)),
            "all preview ink must be vivid red: {colors:?}"
        );
    }
}
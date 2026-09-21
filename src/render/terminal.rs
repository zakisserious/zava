use crate::config::{Config, Orientation, XAxis};
use crate::render::color::ColorMap;
use crate::render::effective_bar_width;
use crossterm::terminal;
use std::io::{self, BufWriter, Stdout, Write};

// ---- sub-character block glyphs (1/8-filled increments) -----------------
//
// Every orientation expresses bar amplitude as one of eight discrete levels
// 1..=8 where `level` means "level/8ths of the cell is ink". The glyphs are
// dyadic fractions of a cell (1/8, 2/8, …, 8/8 = full). Each orientation has
// its own set because the baseline direction changes which side of the cell
// the ink must sit on.
//
// For left/right (horizontal bars) and bottom (vertical bars) every level
// 1..8 has a dedicated, correctly-anchored glyph in Unicode, so we get
// smooth 1/8-increments across the full range.
//
// For top (vertical bars growing DOWN) and right (horizontal bars growing
// LEFT) Unicode only defines exact glyphs for a few levels. Where an exact
// glyph is missing we use the nearest level whose ink sits on the correct
// side of the cell — that means top-anchored chars for top bars (▔ ▕ ▀ █)
// and right-anchored chars for right bars (▐ █). The resulting quantisation
// error is at most 3/8 of one cell at the single partial row closest to the
// bar's lead edge, which is invisible in a spectrum visualisation.
//
// Bottom-anchored (baseline at CELL BOTTOM, bar fills UP):
//   1/8 → ▁   2/8 → ▂   3/8 → ▃   4/8 → ▄
//   5/8 → ▅   6/8 → ▆   7/8 → ▇   8/8 → █
pub(crate) const VERT_CHARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

// Top-anchored (baseline at CELL TOP, bar grows DOWN from the top edge).
//
// Unicode only defines three *top-anchored* block glyphs: ▔ (1/8), ▀ (1/2) and
// █ (full). The intermediate eighths (3/8, 5/8, 6/8, 7/8) simply do not exist
// as top-of-cell fills, so unlike the bottom-anchored set this table is
// quantised to the nearest available glyph that puts its ink on the TOP of the
// cell. Snapping 2/8 and 3/8 up to the half-block keeps the bar's silhouette
// monotonic (a taller bar always covers at least as much ink); using a
// bottom-anchored or left-anchored glyph here would put the ink on the wrong
// edge of the cell and make `top` look broken.
//   1/8 → ▔   2/8 → ▀   3/8 → ▀   4/8 → ▀
//   5/8 → ▀   6/8 → █   7/8 → █   8/8 → █
pub(crate) const TOP_CHARS: [char; 8] = ['▔', '▀', '▀', '▀', '▀', '█', '█', '█'];

// Left-anchored (bar grows RIGHT from the left edge). Every level 1..8 has
// an exact left-anchored glyph.
//   1/8 → ▏   2/8 → ▎   3/8 → ▍   4/8 → ▌
//   5/8 → ▋   6/8 → ▊   7/8 → ▉   8/8 → █
pub(crate) const HORIZ_LEFT_CHARS: [char; 8] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉', '█'];

// Right-anchored (bar grows LEFT from the right edge). Exact glyphs exist
// only for 4/8 (▐ right half) and 8/8 (█ full). The rest are quantised to
// the nearest right-anchored glyph.
//   1/8 → ▐   2/8 → ▐   3/8 → ▐   4/8 → ▐
//   5/8 → ▐   6/8 → █   7/8 → █   8/8 → █
pub(crate) const HORIZ_RIGHT_CHARS: [char; 8] = ['▐', '▐', '▐', '▐', '▐', '█', '█', '█'];

// ASCII fallback (density-only, used when `ascii_glyphs` is on for fonts without
// block glyphs). All orientations share this table — the ink side is lost, but
// the silhouette reads clearly and every level stays monotonic and non-blank.
pub(crate) const ASCII_CHARS: [char; 8] = ['.', ':', ':', '+', '+', '*', '#', '#'];

/// ASCII fallback for the horizontal orientations (grows from the anchor edge).
pub(crate) const HORIZ_ASCII_CHARS: [char; 8] = ['-', ':', ':', '=', '=', '*', '#', '#'];

/// Placement of `n` bars along one axis of `dim` cells.
///
/// Guarantees:
///   * no bar ever leaves the axis, so a layout can never overflow the
///     terminal width and wrap onto the next line (the "overlapping bars"
///     bug), no matter what `bars`/`bar_width`/`bar_spacing` say;
///   * every inter-bar gap is exactly `spacing`; the leftover cells become
///     centered axis-edge margins, so spacing stays uniform as the terminal
///     is resized (previously spare cells were folded into the outermost
///     gaps, making the absolute left/right edges alternate between 1- and
///     2-cell gaps on every resize).
pub(crate) struct BarLayout {
    bar_width: usize,
    /// Axis cell each bar starts at.
    starts: Vec<usize>,
    /// Axis cell -> (bar index, offset inside the bar); `None` = gap.
    rev: Vec<Option<(usize, usize)>>,
}

impl BarLayout {
    pub(crate) fn new(dim: usize, n: usize, bar_width: usize, spacing: usize) -> Self {
        let dim = dim.max(1);
        let bar_width = bar_width.max(1).min(dim);
        let spacing = spacing.min(dim);
        // Largest count whose base footprint (bars + minimum gaps) fits.
        let fit = ((dim + spacing) / (bar_width + spacing)).max(1);
        let n = n.max(1).min(fit);
        let gaps = n - 1;
        let block = n * bar_width + gaps * spacing;
        let leftover = dim.saturating_sub(block);
        // Uniform gaps; the leftover becomes centered edge margins. With
        // `spacing == 0` the block is bars packed edge-to-edge and the spare
        // cells simply sit at the axis edges (never inside the block, so a
        // stereo pair can't leave a permanently missing bar mid-stack).
        let margin = leftover / 2;

        let mut starts = Vec::with_capacity(n);
        let mut pos = margin;
        for i in 0..n {
            starts.push(pos);
            if i + 1 < n {
                pos += bar_width + spacing;
            }
        }

        let mut rev = vec![None; dim];
        for (i, &start) in starts.iter().enumerate() {
            for off in 0..bar_width {
                rev[start + off] = Some((i, off));
            }
        }

        Self { bar_width, starts, rev }
    }

    pub(crate) fn len(&self) -> usize {
        self.starts.len()
    }

    /// Blank cells before bar `i` (includes the leading margin for i == 0).
    pub(crate) fn gap_before(&self, i: usize) -> usize {
        if i == 0 {
            self.starts[0]
        } else {
            self.starts[i] - self.starts[i - 1] - self.bar_width
        }
    }

    /// Blank cells after the last bar.
    #[allow(dead_code)]
    fn trailing(&self, dim: usize) -> usize {
        dim.saturating_sub(self.starts[self.starts.len() - 1] + self.bar_width)
    }
}

pub struct TerminalRenderer {
    writer: BufWriter<Stdout>,
    width: u16,
    height: u16,
    color_map: ColorMap,
    peaks: Vec<f64>,
    /// Transient overlay shown in the bottom-right corner after a key-driven
    /// change (e.g. Left/Right bar count) so the effect is visible even when
    /// the layout itself barely moves. Auto-expires.
    status: Option<(String, std::time::Instant)>,
    /// ASCII glyph fallback for fonts without block characters.
    ascii: bool,
    /// Beat-pulse momentum (0..1) when `beat_pulse` is enabled, decays each
    /// frame after a bass transient fires it.
    beat: f64,
    /// Smoothed bass energy used to detect transients (a slow attack, a jump
    /// past ~1.4x the running level fires the pulse).
    bass_energy: f64,
}

/// How long the status overlay stays on screen.
const STATUS_TTL: std::time::Duration = std::time::Duration::from_millis(1500);

/// Eighths of ink remaining in a mirrored-mode cell, given the bar's total
/// height in eighths and how many rows it is from its half's
/// centre-adjacent base row. Shared by both halves so they cannot drift
/// apart.
pub(crate) fn mirrored_level(sub_h: f64, offset: usize) -> usize {
    ((sub_h - offset as f64 * 8.0).max(0.0) as usize).min(8)
}

/// Glyph for one mirrored-mode cell. The halves are asymmetric by design:
/// bars above the centre grow up (ink at the cell bottom → full 8-level
/// bottom-anchored set), bars below grow down (ink at the cell top → the
/// 3-level top-anchored set, the finest Unicode offers).
pub(crate) fn mirrored_glyph(is_upper: bool, level: usize, idle_here: bool, ascii: bool) -> char {
    let (vert, top) = if ascii {
        (ASCII_CHARS, ASCII_CHARS)
    } else {
        (VERT_CHARS, TOP_CHARS)
    };
    if is_upper {
        if idle_here && level == 0 {
            vert[0]
        } else if level >= 1 {
            vert[level - 1]
        } else {
            ' '
        }
    } else if idle_here && level == 0 {
        if ascii {
            '.'
        } else {
            '▔'
        }
    } else if level >= 1 {
        top[(level - 1).min(7)]
    } else {
        ' '
    }
}

impl TerminalRenderer {
    pub fn new(config: &Config) -> io::Result<Self> {
        let (width, height) = terminal::size().unwrap_or((80, 24));
        let writer = BufWriter::with_capacity(65536, io::stdout());
        let color_map = ColorMap::new(&config.color, height as usize, 64);

        Ok(Self {
            writer,
            width,
            height,
            color_map,
            peaks: vec![0.0; 64],
            status: None,
            ascii: config.output.ascii_glyphs,
            beat: 0.0,
            bass_energy: 0.0,
        })
    }

    /// Show a transient message in the bottom-right corner.
    pub fn set_status(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), std::time::Instant::now()));
    }

    /// Current terminal size in cells, as of the last render or resize.
    pub fn dims(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    pub fn resize(&mut self, config: &Config, new_width: u16, new_height: u16, num_bars: usize) {
        self.width = new_width;
        self.height = new_height;
        self.color_map = ColorMap::new(&config.color, new_height as usize, num_bars);
        self.peaks.resize(num_bars, 0.0);
        self.ascii = config.output.ascii_glyphs;
    }

    pub fn reload_colors(&mut self, config: &Config, num_bars: usize) {
        self.color_map = ColorMap::new(&config.color, self.height as usize, num_bars);
        self.ascii = config.output.ascii_glyphs;
    }

    pub fn render(
        &mut self,
        bars: &[f64],
        config: &Config,
        cut_off_frequencies: Option<&[f64]>,
    ) -> io::Result<()> {
        let (cur_w, cur_h) = terminal::size().unwrap_or((self.width, self.height));
        if cur_w != self.width || cur_h != self.height {
            self.resize(config, cur_w, cur_h, bars.len());
        }

        let width = self.width as usize;
        let mut height = self.height as usize;
        if height == 0 || width == 0 {
            return Ok(());
        }

        let bar_spacing = config.general.bar_spacing;
        let num_bars = bars.len();
        if num_bars == 0 {
            return Ok(());
        }

        // A waveform is a continuous trace, not a set of gapped bars, so it
        // neither reserves a row for the frequency axis nor uses the bar
        // layout at all.
        let waveform = config.output.waveform;

        // Space for the frequency axis and/or a plain baseline ruler, below
        // the bars. Extra rows are only taken when there is room for them.
        let has_xaxis = config.output.xaxis == XAxis::Frequency && height > 3 && !waveform;
        let has_ruler = config.output.baseline_ruler && height > 3 && !waveform;
        let extra_rows = usize::from(has_xaxis) + usize::from(has_ruler);
        let (has_xaxis, has_ruler, extra_rows) =
            if height > extra_rows { (has_xaxis, has_ruler, extra_rows) } else { (false, false, 0) };
        height -= extra_rows;

        let vertical = matches!(
            config.output.orientation,
            Orientation::Left | Orientation::Right
        );
        let n = num_bars;
        let axis = if vertical { height } else { width };

        // In auto mode a too-wide `bar_width` is clamped so the screen always
        // keeps at least a handful of bars instead of a single wall.
        let bar_width = effective_bar_width(config, axis);

        let layout = BarLayout::new(axis, n, bar_width, bar_spacing);

        // Peak-hold/decay. `reduce_motion` disables the trailing marker: peaks
        // just track the current bars, so nothing animates.
        if waveform {
            // A waveform is an instantaneous amplitude trace, not a decaying
            // spectrum; running the peak-hold logic on it leaves stale markers
            // drifting over the trace.
            self.peaks.iter_mut().for_each(|p| *p = 0.0);
        } else if self.peaks.len() != num_bars {
            self.peaks.resize(num_bars, 0.0);
        }
        if !waveform {
            let decay = config.output.peak_decay.max(0.001);
            for (i, &val) in bars.iter().enumerate() {
                if val > self.peaks[i] || config.output.reduce_motion {
                    self.peaks[i] = val;
                } else {
                    let next = self.peaks[i] - decay;
                    // Once the marker is within one decay step of the baseline, snap it
                    // down onto the bar. Pure subtraction converges asymptotically and
                    // would otherwise leave a peak dot stranded on screen forever.
                    self.peaks[i] = if next < decay { val.max(0.0) } else { next.max(val) };
                }
            }
        }

        // Beat-pulse transient detection on the low quarter of the spectrum
        // (no-op for the waveform, whose trace has no bass bins).
        if config.output.beat_pulse && !config.output.reduce_motion && !waveform {
            let quarter = (num_bars / 4).max(1);
            let bass = bars
                .iter()
                .take(quarter)
                .sum::<f64>()
                / quarter as f64;
            if bass > self.bass_energy * 1.5 + 0.015 && bass > 0.02 {
                self.beat = 1.0;
            }
            self.bass_energy += 0.25 * (bass - self.bass_energy);
        } else {
            self.beat = 0.0;
            self.bass_energy = 0.0;
        }
        self.beat *= 0.86;

        // Begin synchronized update (reduces tearing in modern terminals)
        let _ = write!(self.writer, "\x1b[?2026h\x1b[H");

        // Set default background
        let _ = write!(self.writer, "{}", self.color_map.bg_ansi);

        if waveform {
            // Oscilloscope mode: one sample per column mapped to a vertical
            // position, joined into a continuous trace. It deliberately
            // ignores the bar layout — a scope has no gaps and no idle heads.
            self.render_waveform(bars, config, height, width)?;
        } else {
            match config.output.orientation {
                Orientation::Bottom => {
                    self.render_bottom(bars, config, height, width, &layout)?;
                }
                Orientation::Top => {
                    self.render_top(bars, config, height, width, &layout)?;
                }
                Orientation::Left => {
                    self.render_left(bars, config, height, width, &layout)?;
                }
                Orientation::Right => {
                    self.render_right(bars, config, height, width, &layout)?;
                }
                Orientation::Horizontal => {
                    // The centred, mirrored spectrum: bars grow up AND down
                    // from a middle line, matching the documented meaning of
                    // this orientation.
                    self.render_mirrored(bars, config, height, width, &layout)?;
                }
            }
        }

        // Stereo channel divider: a faint `│` in the gap between the two
        // channel halves of a non-split stereo layout. Uses the *laid-out*
        // bar count (`layout.len()`), which may be tighter than `num_bars`
        // when the width can't fit the requested count.
        if !waveform
            && config.output.stereo_divider
            && config.output.channels == 2
            && !config.output.split_stereo
            && layout.len() % 2 == 0
            && !vertical
        {
            let half = layout.len() / 2;
            if half > 0 && layout.gap_before(half) > 0 {
                let col = layout.starts[half] - 1 - layout.gap_before(half) / 2;
                let is_gap = layout.rev.get(col).is_none_or(|v| v.is_none());
                if col < width && is_gap {
                    self.render_vertical_divider(col, height)?;
                }
            }
        }

        // A faint full-width baseline under the bars (typed below the spectrum
        // when there is no numeric x-axis to fill the row).
        if has_ruler {
            self.render_baseline(width)?;
        }

        // Render x-axis if enabled
        if has_xaxis {
            self.render_xaxis(&layout, cut_off_frequencies)?;
        }

        // Transient status overlay (bottom-right). Written after the frame so
        // the next full redraw erases it once it expires.
        let show_status = self
            .status
            .as_ref()
            .is_some_and(|(_, t)| t.elapsed() < STATUS_TTL);
        if show_status {
            if let Some((msg, _)) = &self.status {
                let col = width.saturating_sub(msg.chars().count()) + 1;
                let _ = write!(
                    self.writer,
                    "\x1b[{row};{col}H\x1b[7m{msg}\x1b[0m",
                    row = height + 1 + usize::from(has_xaxis) + usize::from(has_ruler),
                    col = col.max(1),
                );
            }
        } else {
            self.status = None;
        }

        // Beat pulse: a full-width bright flash across the top row while the
        // bass momentum is alive. Written last (before the reset) so it cannot
        // shift the rows laid out above.
        if self.beat > 0.06 {
            let w = (self.beat * 200.0 + 30.0).round() as u8;
            let _ = write!(
                self.writer,
                "\x1b[1;1H\x1b[38;2;{w};{w};{}m{}",
                (self.beat * 235.0 + 20.0) as u8,
                "\u{2588}".repeat(width),
            );
        }

        // Reset color and end synchronized update
        let _ = write!(self.writer, "\x1b[0m\x1b[?2026l");
        self.writer.flush()?;
        Ok(())
    }

/// Oscilloscope trace, in two flavours:
///
/// * `line` (default): a thin continuous trace using half-block glyphs
///   (`▀` `▄` `█`) — the same glyphs CAVA-family tools rely on, safe in every
///   terminal font, two vertical levels per cell.
/// * `area`: fills from the screen bottom up to the waveform with the full
///   8-level sub-character set (`VH` [`VERT_CHARS`]), which reads far smoother
///   and gives the classic solid-scope look.
///
/// Optional guides: a faint `0.5` baseline (the trace's zero line) and a
/// quarter/three-quarter graticule, drawn only in blank cells so the trace
/// always wins. `waveform_dynamics` additionally draws a dim local
/// min/max envelope band hugging the line.
///
/// `trace` holds one value per column in `[0, 1]` where 0.5 is the
/// baseline (silence), exactly as produced by the waveform branch in
/// `main.rs`.
fn render_waveform(
    &mut self,
    trace: &[f64],
    config: &Config,
    height: usize,
    width: usize,
) -> io::Result<()> {
    if height == 0 || width == 0 {
        return Ok(());
    }
    // Horizontal resolution is one column; vertical is two half-rows per cell.
    let scale = config.general.max_height.clamp(0.05, 1.0);
    let half_max = ((height * 2 - 1) as f64 * scale).round().max(1.0) as usize;
    let to_half = |v: f64| -> usize {
        (((1.0 - v.clamp(0.0, 1.0)) * half_max as f64).round() as usize).min(half_max)
    };

    let area = config.output.waveform_style == crate::config::WaveformStyle::Area;
    let baseline = config.output.waveform_baseline;
    let graticule = config.output.waveform_graticule;
    let dynamics = config.output.waveform_dynamics && !area;

    let vert = if self.ascii { ASCII_CHARS } else { VERT_CHARS };

    // Per-column data: thin line spans, optional dim envelope band, and the
    // area-style fill boundary (in half-rows from the top, real-valued so the
    // 8-level glyphs land inside the cell instead of rounding to the thin
    // line's resolution).
    let spans = if area { Vec::new() } else { waveform_cells(trace, width, half_max) };
    let dyns = if dynamics { envelope_cells(trace, width, half_max, 2) } else { Vec::new() };
    let area_b: Vec<f64> = if area {
        trace
            .iter()
            .map(|v| (1.0 - v.clamp(0.0, 1.0)) * half_max as f64)
            .collect()
    } else {
        Vec::new()
    };

    let trace_esc = self.color_map.get_fg_escape(0, 0);
    let guide_esc = self.color_map.dim_fg(0, 0, 0.32);
    let dyn_esc = self.color_map.dim_fg(0, 0, 0.20);

    // Guideline half-rows: the 0.5 baseline and optional quarter graticule.
    let base_g = to_half(0.5);
    let mut guides: Vec<usize> = Vec::new();
    if baseline {
        guides.push(base_g);
    }
    if graticule {
        for f in [0.25f64, 0.75f64] {
            let g = to_half(f);
            if g != base_g && !guides.contains(&g) {
                guides.push(g);
            }
        }
    }

    for y in 0..height {
        let mut line = String::with_capacity(width + 64);
        let mut cur = String::new();
        let upper = y * 2;
        let lower = upper + 1;
        let row_guide = guides.iter().copied().find(|&g| g / 2 == y);

        for x in 0..width {
            if area {
                let b = area_b[x];
                let fill = (2.0 * (y + 1) as f64 - b).clamp(0.0, 2.0);
                if fill > 0.0 {
                    let level = (fill * 4.0).round() as usize;
                    let esc = self.color_map.get_fg_escape(y, x);
                    if esc != cur {
                        line.push_str(&esc);
                        cur = esc;
                    }
                    line.push(vert[level.clamp(1, 8) - 1]);
                    continue;
                }
            } else {
                let (lo, hi) = spans[x];
                let up = lo <= upper && upper <= hi;
                let dwn = lo <= lower && lower <= hi;
                if up || dwn {
                    let ch = if self.ascii {
                        if up && dwn { '#' } else if dwn { ',' } else { ':' }
                    } else if up && dwn {
                        '\u{2588}'
                    } else if up {
                        '\u{2580}'
                    } else {
                        '\u{2584}'
                    };
                    if trace_esc != cur {
                        line.push_str(&trace_esc);
                        cur = trace_esc.clone();
                    }
                    line.push(ch);
                    continue;
                }
            }

            // Dim envelope band, only where the trace left the cell blank.
            if !dyns.is_empty() {
                let (dlo, dhi) = dyns[x];
                let up = dlo <= upper && upper <= dhi;
                let dwn = dlo <= lower && lower <= dhi;
                if up || dwn {
                    let ch = if self.ascii {
                        '='
                    } else if up && dwn {
                        '\u{2588}'
                    } else if up {
                        '\u{2580}'
                    } else {
                        '\u{2584}'
                    };
                    if dyn_esc != cur {
                        line.push_str(&dyn_esc);
                        cur = dyn_esc.clone();
                    }
                    line.push(ch);
                    continue;
                }
            }

            // Guideline tick on this cell's half-row.
            if let Some(g) = row_guide {
                let in_cell = (g % 2 == 0 && upper == g) || (g % 2 == 1 && lower == g);
                if in_cell {
                    if guide_esc != cur {
                        line.push_str(&guide_esc);
                        cur = guide_esc.clone();
                    }
                    line.push(if g == base_g { '\u{2500}' } else { '\u{00b7}' });
                    continue;
                }
            }

            line.push(' ');
        }

        if y + 1 < height {
            line.push_str("\r\n");
        } else {
            line.push('\r');
        }
        write!(self.writer, "{line}")?;
    }
    Ok(())
}

    /// Mirrored spectrum ("horizontal" orientation).
    ///
    /// The bars grow **up and down from a centre line**. Both halves use the
    /// same `mirrored_level` distance math so a given amplitude produces the
    /// same amount of ink above and below the centre, but the *glyph tables*
    /// differ: bars above the centre grow up, so their cells are drawn with
    /// the full 8-level bottom-anchored set, while bars below the centre grow
    /// down with the 3-level top-anchored set (Unicode defines no top fills
    /// finer than ▔ ▀ █). The halves are not pixel mirrors of each other —
    /// the lower half carries the detail and the upper half the silhouette.
    /// A neighbour half-cell rounding scheme was tried before and looked
    /// blockier for no visible gain.
    ///
    /// Only universally-supported glyphs are used (`▄` `▀` `█`), and for odd
    /// terminal heights the spare row ends up at the top of the screen so the
    /// block stays anchored to the bottom edge.
    fn render_mirrored(
        &mut self,
        bars: &[f64],
        config: &Config,
        height: usize,
        width: usize,
        layout: &BarLayout,
    ) -> io::Result<()> {
        let half = (height / 2).max(1);
        let half_max = (half as f64 * 8.0 * config.general.max_height).max(1.0);
        // y counts from the bottom screen row. The centre line sits between
        // `lower_base` and `upper_base`; for odd heights the row above the
        // upper half's reach stays blank so the bottom edge is always filled.
        let upper_base = half;
        let lower_base = half - 1;
        let idle = config.output.show_idle_bar_heads;
        let accent_on = config.output.bright_peaks || config.output.bar_cap;
        let mut last_color = String::new();

        for y in (0..height).rev() {
            let mut line = String::with_capacity(width + 64);
            if let Some(bg) = self.color_map.bg_row(y) {
                line.push_str(bg);
            }

            for (b_idx, &bar_norm) in bars.iter().enumerate() {
                let gap = layout.gap_before(b_idx);
                if gap > 0 {
                    line.push_str(&" ".repeat(gap));
                }

                let sub_h = bar_norm * half_max;
                // How far this row sits from the centre-adjacent base row of
                // its own half. Both halves measure that distance the same way,
                // so `mirrored_level` yields identical ink for identical
                // amplitudes and the two sides cannot drift apart.
                let (offset, is_upper) = if y >= upper_base {
                    (y - upper_base, true)
                } else {
                    (lower_base - y, false)
                };
                let level = mirrored_level(sub_h, offset);
                let idle_here = offset == 0 && idle;
                let mut glyph = mirrored_glyph(is_upper, level, idle_here, self.ascii);

                // Peak trail, drawn only where the bar itself leaves this band
                // of the row blank — the `_` sits one cell beyond the decaying
                // bar tip, the same rule the bottom orientation uses.
                if glyph == ' ' && config.output.draw_peaks {
                    let peak_sub_h = self.peaks[b_idx] * half_max;
                    let residual = peak_sub_h - offset as f64 * 8.0;
                    if (0.0..8.0).contains(&residual) {
                        glyph = '_';
                    }
                }

                // Mirror the colour gradient around the centre line too. Rows
                // beyond the mirrored range have no ink, but clamp anyway so
                // the index can never underflow.
                let colour_row = if is_upper {
                    (2 * half - 1).saturating_sub(y).min(height - 1)
                } else {
                    y
                };

                if glyph != ' ' {
                    // Highlight the peak trail and the partial lead cell (the
                    // bar cap) in the high-contrast accent ink.
                    let is_peak = glyph == '_';
                    let is_cap = config.output.bar_cap && !is_peak && (1..8).contains(&level) && !(idle_here && level == 0);
                    let col_esc = if accent_on && (is_peak || is_cap) {
                        self.color_map.accent_escape().to_string()
                    } else {
                        self.color_map.get_fg_escape(colour_row, b_idx)
                    };
                    if col_esc != last_color {
                        line.push_str(&col_esc);
                        last_color = col_esc;
                    }
                }

                for _ in 0..layout.bar_width {
                    line.push(glyph);
                }
            }

            let printed_len = layout.starts[layout.len() - 1] + layout.bar_width;
            if printed_len < width {
                line.push_str(&" ".repeat(width - printed_len));
            }

            if y > 0 {
                line.push_str("\r\n");
            } else {
                line.push('\r');
            }
            write!(self.writer, "{line}")?;
        }

        Ok(())
    }

    fn render_bottom(
        &mut self,
        bars: &[f64],
        config: &Config,
        height: usize,
        width: usize,
        layout: &BarLayout,
    ) -> io::Result<()> {
        // `bar_reflection` mirrors the spectrum downward from the baseline: the
        // real bars occupy the top two-thirds and a dim inverted copy grows
        // down the bottom third, the analog-scope look. The reflection is only
        // meaningful where bars bottom-anchor, so it lives here.
        let refl = config.output.bar_reflection && height >= 6;
        let main_h = if refl { (height * 2 / 3).max(1) } else { height };
        let refl_h = height.saturating_sub(main_h);
        let vert = if self.ascii { ASCII_CHARS } else { VERT_CHARS };
        let accent_on = config.output.bright_peaks || config.output.bar_cap;

        let max_sub_height = (main_h as f64 * 8.0 * config.general.max_height).max(1.0);
        let mut last_color = String::new();

        // Render from the top of the main region down to its base line
        // (y = 0 in this loop is the baseline the bars grow up from).
        for y in (0..main_h).rev() {
            let mut line = String::with_capacity(width + 64);
            if let Some(bg) = self.color_map.bg_row(y) {
                line.push_str(bg);
            }

            for (b_idx, &bar_norm) in bars.iter().enumerate() {
                let gap = layout.gap_before(b_idx);
                if gap > 0 {
                    line.push_str(&" ".repeat(gap));
                }

                let sub_h = bar_norm * max_sub_height;
                let cell_val = sub_h - (y as f64 * 8.0);

                let mut char_to_print = if cell_val >= 8.0 {
                    vert[7]
                } else if cell_val >= 1.0 {
                    let idx = (cell_val as usize).clamp(1, 8) - 1;
                    vert[idx]
                } else if y == 0 && config.output.show_idle_bar_heads {
                    vert[0]
                } else {
                    ' '
                };

                let mut is_peak = false;
                if config.output.draw_peaks && char_to_print == ' ' {
                    let peak_sub_h = self.peaks[b_idx] * max_sub_height;
                    let peak_cell_val = peak_sub_h - (y as f64 * 8.0);
                    if (0.0..8.0).contains(&peak_cell_val) {
                        char_to_print = '_';
                        is_peak = true;
                    }
                }
                let is_cap =
                    config.output.bar_cap && !is_peak && (1.0..8.0).contains(&cell_val);

                if char_to_print != ' ' {
                    let col_esc = if accent_on && (is_peak || is_cap) {
                        self.color_map.accent_escape().to_string()
                    } else {
                        self.color_map.get_fg_escape(y, b_idx)
                    };
                    if col_esc != last_color {
                        line.push_str(&col_esc);
                        last_color = col_esc;
                    }
                }

                for _ in 0..layout.bar_width {
                    line.push(char_to_print);
                }
            }

            // Fill rest of terminal width with spaces
            let printed_len = layout.starts[layout.len() - 1] + layout.bar_width;
            if printed_len < width {
                line.push_str(&" ".repeat(width - printed_len));
            }

            // With a reflection there is another region below, so the base row
            // ends in a newline; without it the base row is the screen bottom
            // and ends with a carriage return (matching the other renderers).
            if y > 0 || refl {
                line.push_str("\r\n");
            } else {
                line.push('\r');
            }
            write!(self.writer, "{line}")?;
        }

        // The dim inverted reflection below the baseline. Row `r` is `r` cells
        // below the baseline; a bar that filled the whole main height shows a
        // matching column of ink heading downward.
        if refl {
            for r in 0..refl_h {
                let mut line = String::with_capacity(width + 64);
                for (b_idx, &bar_norm) in bars.iter().enumerate() {
                    let gap = layout.gap_before(b_idx);
                    if gap > 0 {
                        line.push_str(&" ".repeat(gap));
                    }
                    let sub_h = bar_norm * max_sub_height;
                    let cell_val = sub_h - (r as f64 * 8.0);
                    let ch = if cell_val >= 8.0 {
                        vert[7]
                    } else if cell_val >= 1.0 {
                        vert[(cell_val as usize).clamp(1, 8) - 1]
                    } else {
                        ' '
                    };
                    if ch != ' ' {
                        // Mirror the colour index around the baseline and dim it.
                        let src_y = (main_h - 1).saturating_sub(r).min(main_h - 1);
                        let col_esc = self.color_map.dim_fg(src_y, b_idx, 0.40);
                        if col_esc != last_color {
                            line.push_str(&col_esc);
                            last_color = col_esc;
                        }
                    }
                    for _ in 0..layout.bar_width {
                        line.push(ch);
                    }
                }
                let printed_len = layout.starts[layout.len() - 1] + layout.bar_width;
                if printed_len < width {
                    line.push_str(&" ".repeat(width - printed_len));
                }
                if !(r + 1 == refl_h) {
                    line.push_str("\r\n");
                } else {
                    line.push('\r');
                }
                write!(self.writer, "{line}")?;
            }
        }

        Ok(())
    }

    fn render_top(
        &mut self,
        bars: &[f64],
        config: &Config,
        height: usize,
        width: usize,
        layout: &BarLayout,
    ) -> io::Result<()> {
        // Top bars grow DOWN from row 0. The table has been quantised to the
        // nearest available top-anchored glyph for each level 1..8, so we
        // index directly by `(level-1)` and no longer need to walk the table
        // index-by-index.
        let max_sub_height = (height as f64 * 8.0 * config.general.max_height).max(1.0);
        let top = if self.ascii { ASCII_CHARS } else { TOP_CHARS };
        let accent_on = config.output.bright_peaks || config.output.bar_cap;
        let mut last_color = String::new();

        for y in 0..height {
            let mut line = String::with_capacity(width + 64);
            if let Some(bg) = self.color_map.bg_row(height - 1 - y) {
                line.push_str(bg);
            }

            for (b_idx, &bar_norm) in bars.iter().enumerate() {
                let gap = layout.gap_before(b_idx);
                if gap > 0 {
                    line.push_str(&" ".repeat(gap));
                }

                // The number of eighths of a cell, above row `y`, that this
                // bar's ink covers.  Rounded *down* so we never over-paint.
                let level = ((bar_norm * max_sub_height - (y as f64 * 8.0)).max(0.0)
                    as usize)
                    .min(8);
                let mut char_to_print = if level >= 8 {
                    top[7] // full cell
                } else if level >= 1 {
                    top[level - 1]
                } else if y == 0 && config.output.show_idle_bar_heads {
                    top[0] // ▔ cap row
                } else {
                    ' '
                };

                let mut is_peak = false;
                if config.output.draw_peaks
                    && char_to_print == ' '
                    && self.peaks[b_idx] * max_sub_height >= y as f64 * 8.0
                {
                    char_to_print = if self.ascii { 'v' } else { '▾' };
                    is_peak = true;
                }
                let is_cap =
                    config.output.bar_cap && !is_peak && (1..8).contains(&level);

                if char_to_print != ' ' {
                    let col_esc = if accent_on && (is_peak || is_cap) {
                        self.color_map.accent_escape().to_string()
                    } else {
                        self.color_map.get_fg_escape(height - 1 - y, b_idx)
                    };
                    if col_esc != last_color {
                        line.push_str(&col_esc);
                        last_color = col_esc;
                    }
                }

                for _ in 0..layout.bar_width {
                    line.push(char_to_print);
                }
            }

            let printed_len = layout.starts[layout.len() - 1] + layout.bar_width;
            if printed_len < width {
                line.push_str(&" ".repeat(width
                    .saturating_sub(printed_len)));
            }

            if y + 1 < height {
                line.push_str("\r\n");
            } else {
                line.push('\r');
            }
            write!(self.writer, "{line}")?;
        }

        Ok(())
    }

    fn render_left(
        &mut self,
        bars: &[f64],
        config: &Config,
        height: usize,
        width: usize,
        layout: &BarLayout,
    ) -> io::Result<()> {
        // Bars grow RIGHT from the leftmost edge of each bar's cell block.
        // HORIZ_LEFT_CHARS is fully populated at every level 1..8, so we
        // index `(level-1)` directly.
        let max_sub_width = (width as f64 * 8.0 * config.general.max_height).max(1.0);
        let horiz = if self.ascii { HORIZ_ASCII_CHARS } else { HORIZ_LEFT_CHARS };
        let accent_on = config.output.bright_peaks || config.output.bar_cap;
        let mut last_color = String::new();

        for y in 0..height {
            let mut line = String::with_capacity(width + 64);
            if let Some(bg) = self.color_map.bg_row(y) {
                line.push_str(bg);
            }

            if let Some((b_idx, _)) = layout.rev[y] {
                let bar_norm = bars[b_idx];

                for x in 0..width {
                    let cell_val = (bar_norm * max_sub_width) - (x as f64 * 8.0);
                    let cell_level = (cell_val.max(0.0) as usize).min(8);
                    let mut char_to_print = if cell_level >= 8 {
                        horiz[7]
                    } else if cell_level >= 1 {
                        horiz[cell_level - 1]
                    } else if x == 0 && config.output.show_idle_bar_heads {
                        horiz[0]
                    } else {
                        ' '
                    };

                    let mut is_peak = false;
                    if config.output.draw_peaks && char_to_print == ' ' {
                        let peak_sub_w = self.peaks[b_idx] * max_sub_width;
                        let peak_cell_val = peak_sub_w - (x as f64 * 8.0);
                        if (0.0..8.0).contains(&peak_cell_val) {
                            char_to_print = if self.ascii { '!' } else { '|' };
                            is_peak = true;
                        }
                    }
                    let is_cap =
                        config.output.bar_cap && !is_peak && (1..8).contains(&cell_level);

                    if char_to_print != ' ' {
                        let col_esc = if accent_on && (is_peak || is_cap) {
                            self.color_map.accent_escape().to_string()
                        } else {
                            self.color_map.get_fg_escape(y, b_idx)
                        };
                        if col_esc != last_color {
                            line.push_str(&col_esc);
                            last_color = col_esc;
                        }
                    }
                    line.push(char_to_print);
                }
            } else {
                line.push_str(&" ".repeat(width));
            }

            if y + 1 < height {
                line.push_str("\r\n");
            } else {
                line.push('\r');
            }
            write!(self.writer, "{line}")?;
        }
        Ok(())
    }

    fn render_right(
        &mut self,
        bars: &[f64],
        config: &Config,
        height: usize,
        width: usize,
        layout: &BarLayout,
    ) -> io::Result<()> {
        // Bars grow LEFT from the rightmost edge of each bar's cell block.
        // HORIZ_RIGHT_CHARS is quantised to the nearest right-anchored glyph,
        // so we index `(level-1)` directly rather than walking the table
        // index-by-index.
        let max_sub_width = (width as f64 * 8.0 * config.general.max_height).max(1.0);
        let horiz = if self.ascii { HORIZ_ASCII_CHARS } else { HORIZ_RIGHT_CHARS };
        let accent_on = config.output.bright_peaks || config.output.bar_cap;
        let mut last_color = String::new();

        for y in 0..height {
            let mut line = String::with_capacity(width + 64);
            if let Some(bg) = self.color_map.bg_row(y) {
                line.push_str(bg);
            }

            if let Some((b_idx, _)) = layout.rev[y] {
                let bar_norm = bars[b_idx];
                let sub_w = bar_norm * max_sub_width;

                for x in 0..width {
                    let dist_from_right = width - 1 - x;
                    let cell_val = sub_w - (dist_from_right as f64 * 8.0);
                    let level = (cell_val.max(0.0) as usize).min(8);
                    let mut char_to_print = if level >= 8 {
                        horiz[7]
                    } else if level >= 1 {
                        horiz[level - 1]
                    } else if dist_from_right == 0 && config.output.show_idle_bar_heads {
                        horiz[0]
                    } else {
                        ' '
                    };

                    let mut is_peak = false;
                    if config.output.draw_peaks && char_to_print == ' ' {
                        let peak_sub_w = self.peaks[b_idx] * max_sub_width;
                        let peak_cell_val = peak_sub_w - (dist_from_right as f64 * 8.0);
                        if (0.0..8.0).contains(&peak_cell_val) {
                            char_to_print = if self.ascii { '!' } else { '|' };
                            is_peak = true;
                        }
                    }
                    let is_cap =
                        config.output.bar_cap && !is_peak && (1..8).contains(&level);

                    if char_to_print != ' ' {
                        let col_esc = if accent_on && (is_peak || is_cap) {
                            self.color_map.accent_escape().to_string()
                        } else {
                            self.color_map.get_fg_escape(y, b_idx)
                        };
                        if col_esc != last_color {
                            line.push_str(&col_esc);
                            last_color = col_esc;
                        }
                    }
                    line.push(char_to_print);
                }
            } else {
                line.push_str(&" ".repeat(width));
            }

            if y + 1 < height {
                line.push_str("\r\n");
            } else {
                line.push('\r');
            }
            write!(self.writer, "{line}")?;
        }
        Ok(())
    }


    /// Stereo channel divider: a faint vertical `│` marking the gap between
    /// the two channel halves, so the stereo split reads at a glance. Drawn
    /// with absolute cursor addressing, then the cursor is parked back at the
    /// bottom row so the baseline / x-axis rows that follow keep their
    /// relative `\r\n` stepping.
    fn render_vertical_divider(&mut self, col: usize, height: usize) -> io::Result<()> {
        for row in 1..=height {
            write!(self.writer, "\x1b[{row};{}H\x1b[90m│\x1b[0m", col + 1)?;
        }
        write!(self.writer, "\x1b[{height};1H")?;
        Ok(())
    }

    /// Faint full-width baseline ruler typed directly under the spectrum when
    /// there is no numeric x-axis. A dim dashed line reads as a ground line
    /// without competing with the bars.
    fn render_baseline(&mut self, width: usize) -> io::Result<()> {
        let mut line = String::with_capacity(width + 16);
        line.push_str("\r\n\x1b[90m");
        for i in 0..width {
            line.push(if i % 2 == 0 { '─' } else { ' ' });
        }
        line.push_str("\x1b[0m\r");
        write!(self.writer, "{line}")?;
        Ok(())
    }

    fn render_xaxis(
        &mut self,
        layout: &BarLayout,
        cut_off_frequencies: Option<&[f64]>,
    ) -> io::Result<()> {
        let num_bars = layout.len();
        let mut line = String::with_capacity(16);
        line.push_str("\r\n\x1b[90m"); // Gray text for frequencies

        if let Some(freqs) = cut_off_frequencies {
            for i in 0..num_bars {
                let gap = layout.gap_before(i);
                if gap > 0 {
                    line.push_str(&" ".repeat(gap));
                }
                let f = if i < freqs.len() { freqs[i] } else { 0.0 };
                let label = if f >= 1000.0 {
                    format!("{:.1}k", f / 1000.0)
                } else {
                    format!("{}", f as u32)
                };
                // Only write the label if it fits in its own bar slot plus the
                // gap before the next bar; a label spilling into the next
                // bar's slot reads as overlapping text. When it doesn't fit,
                // leave the bar's slot blank rather than skip the gap (skipping
                // the gap shifts every following label off its bar).
                let budget = layout.bar_width
                    + if i + 1 < num_bars {
                        layout.gap_before(i + 1)
                    } else {
                        0
                    };
                if label.len() <= budget {
                    line.push_str(&label);
                } else {
                    line.push_str(
                        &" ".repeat(layout.bar_width),
                    );
                }
            }
        }
        line.push_str("\x1b[0m\r");
        write!(self.writer, "{line}")?;
        Ok(())
    }
}

/// Per-column trace band for one column of the oscilloscope.
///
/// Returns one `(top, bottom)` half-row pair per column of `width`, in the
/// same half-row space that the renderer draws (value 1.0 = top, 0.0 =
/// bottom; `half_max` is the bottommost half-row).
///
/// The trace is drawn as a *line*, not a filled envelope: when the trace has
/// at least one sample per column the band collapses to the single sample
/// nearest the column's centre (a dense waveform would otherwise blanket the
/// column in a solid min/max slab), and neighbouring columns are joined by
/// widening each band to the previous column's band so steep runs stay
/// continuous instead of dotting. When upsampling (fewer trace points than
/// columns) the value is interpolated so the line stays smooth.
///
/// Shared by the terminal renderer and the menu's live preview so the two
/// can never drift apart.
pub(crate) fn waveform_cells(trace: &[f64], width: usize, half_max: usize) -> Vec<(usize, usize)> {
    let n = trace.len();
    if n == 0 || width == 0 || half_max == 0 {
        return Vec::new();
    }
    let to_half = |v: f64| -> usize {
        (((1.0 - v.clamp(0.0, 1.0)) * half_max as f64).round() as usize).min(half_max)
    };
    let mut spans: Vec<(usize, usize)> = Vec::with_capacity(width);
    let mut prev: Option<(usize, usize)> = None;
    for x in 0..width {
        let v = if width <= 1 {
            trace[0]
        } else if n >= width {
            let idx = ((x as f64 * n as f64 / width as f64).round() as usize).min(n - 1);
            trace[idx]
        } else {
            let pos = x as f64 * (n - 1) as f64 / (width - 1) as f64;
            let i0 = (pos as usize).min(n - 1);
            let i1 = (i0 + 1).min(n - 1);
            let frac = pos - i0 as f64;
            trace[i0] + (trace[i1] - trace[i0]) * frac
        };
        let h = to_half(v);
        let band = match prev {
            Some((p0, p1)) => (p0.min(h), p1.max(h)),
            None => (h, h),
        };
        spans.push(band);
        prev = Some((h, h));
    }
    spans
}

/// Per-column local min/max band for the `waveform_dynamics` glow: each column
/// takes the extreme trace values within `radius` columns of itself, mapped to
/// a `(top, bottom)` half-row span like [`waveform_cells`]. The band is wider
/// than the thin line, so it reads as a soft pulse envelope behind it.
fn envelope_cells(trace: &[f64], width: usize, half_max: usize, radius: usize) -> Vec<(usize, usize)> {
    let n = trace.len();
    if n == 0 || width == 0 || half_max == 0 {
        return Vec::new();
    }
    let to_half = |v: f64| -> usize {
        (((1.0 - v.clamp(0.0, 1.0)) * half_max as f64).round() as usize).min(half_max)
    };
    (0..width)
        .map(|x| {
            let lo = (x as isize - radius as isize).max(0) as usize;
            let hi = ((x as isize + radius as isize).min((n - 1) as isize)) as usize;
            let (mut mn, mut mx) = (f64::MAX, f64::MIN);
            for v in &trace[lo..=hi] {
                mn = mn.min(*v);
                mx = mx.max(*v);
            }
            // Max value sits at the smallest (topmost) half-row.
            (to_half(mx), to_half(mn))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for the "CAVA looks better" charset bug: the lowest
    /// level must be a visible block glyph, not a space. A blank entry here
    /// silently hid every bar in the bottom 1/8..2/8 of a cell and made
    /// `show_idle_bar_heads` a no-op (CAVA draws `▁` in both cases).
    #[test]
    fn vertical_charset_has_no_blank_level() {
        assert_eq!(VERT_CHARS[0], '\u{2581}', "lowest vertical level must be ▁");
        assert_eq!(VERT_CHARS[7], '█');
        assert!(
            VERT_CHARS.iter().all(|c| *c != ' '),
            "VERT_CHARS must not contain a space glyph"
        );
    }

    /// Mirrored-mode regression guard: a bar with any ink at all must never
    /// render blank. Levels 1..=2 used to snap to `' '`, so quiet frequency
    /// bins vanished and left blank runs across the middle of the
    /// `horizontal` view (measured: 17 blank columns of 100 at normal
    /// levels, while `bottom`/`top` always showed at least `▁`).
    #[test]
    fn mirrored_glyph_never_blank_for_inked_levels() {
        for is_upper in [false, true] {
            assert_eq!(
                mirrored_glyph(is_upper, 0, false, false),
                ' ',
                "true silence should stay blank"
            );
            for level in 1..=8 {
                assert_ne!(
                    mirrored_glyph(is_upper, level, false, false),
                    ' ',
                    "level {level} vanished ({})",
                    if is_upper { "upper" } else { "lower" }
                );
            }
        }
    }

    /// The mirror must be monotonic: a taller bar never draws less ink.
    #[test]
    fn mirrored_glyph_is_monotonic() {
        fn ink(c: char) -> usize {
            match c {
                ' ' => 0,
                '▁' | '▔' => 1,
                '▂' => 2,
                '▃' => 3,
                '▄' | '▀' => 4,
                '▅' => 5,
                '▆' => 6,
                '▇' => 7,
                '█' => 8,
                _ => 0,
            }
        }
        for is_upper in [false, true] {
            let mut prev = 0;
            for level in 0..=8usize {
                let r = ink(mirrored_glyph(is_upper, level, false, false));
                assert!(
                    r >= prev,
                    "mirrored_glyph(upper={is_upper}, level={level}) inked {r} < previous {prev}"
                );
                prev = r;
            }
            assert_eq!(mirrored_glyph(is_upper, 0, false, false), ' ');
            assert_eq!(mirrored_glyph(is_upper, 8, false, false), '\u{2588}');
        }
    }

    /// Every other orientation already used the correct 1/8 block for its
    /// lowest level; make sure that stays true.
    #[test]
    fn other_charsets_have_no_blank_level() {
        assert_eq!(TOP_CHARS[0], '\u{2594}'); // ▔ upper one eighth
        assert_eq!(TOP_CHARS[7], '█');
        assert_eq!(HORIZ_LEFT_CHARS[0], '\u{258F}'); // ▏ left one eighth
        assert_eq!(HORIZ_LEFT_CHARS[7], '█');
        assert_eq!(HORIZ_RIGHT_CHARS[0], '\u{2590}'); // ▐ right one eighth
        assert_eq!(HORIZ_RIGHT_CHARS[7], '█');
        for set in [TOP_CHARS, HORIZ_LEFT_CHARS, HORIZ_RIGHT_CHARS] {
            assert!(
                set.iter().all(|c| *c != ' '),
                "orientation charset must not contain a space glyph"
            );
            assert_eq!(set.len(), 8, "charset must have 8 entries");
        }
    }

    #[test]
    fn charsets_are_monotonically_growing() {
        // Each position must be a distinct glyph so a level maps to exactly one
        // block character — enforced for the fully-populated sets (VERT and
        // HORIZ_LEFT) where every level 1..8 has its own dedicated glyph.
        for set in [VERT_CHARS, HORIZ_LEFT_CHARS] {
            let mut seen = set.to_vec();
            seen.sort();
            seen.dedup();
            assert_eq!(seen.len(), set.len(), "charset contains duplicate glyphs");
        }
        // TOP_CHARS and HORIZ_RIGHT_CHARS are intentionally quantised to the
        // nearest available anchored glyph (only 2 or 3 distinct glyphs across
        // the 8 slots), so we only check that they have no blanks and the
        // endpoints are correct.
        for set in [TOP_CHARS, HORIZ_RIGHT_CHARS] {
            assert!(
                set.iter().all(|c| *c != ' '),
                "quantised charset must not contain a space glyph"
            );
            assert_eq!(set.len(), 8, "quantised charset must have 8 entries");
            assert_ne!(set[0], set[7], "quantised charset must distinguish empty from full");
        }
    }

    /// Regression guard for the asymmetric `horizontal` mode: the two halves used
    /// different glyph tables (eight-level bottom vs three-level top), so one
    /// bar rendered as `▆` above the centre and `█` below it.
    ///
    /// The geometry must pair every upper row with a lower row the same
    /// distance from the centre line, so `mirrored_level` sees the same
    /// `offset` on both sides and the ink mirror is exact.
    #[test]
    fn mirrored_geometry_pairs_rows_symmetrically() {
        for height in [8usize, 9, 10, 24, 25, 40, 51] {
            let half = (height / 2).max(1);
            let upper_base = half;
            let lower_base = half - 1;

            // The two centre-adjacent rows must be exactly one row apart.
            assert_eq!(upper_base - lower_base, 1, "height={height}");

            for y_lower in 0..=lower_base {
                let y_upper = (2 * half - 1).saturating_sub(y_lower);
                if y_upper >= height {
                    continue; // row beyond the upper half's reach (odd heights)
                }
                assert!(
                    y_upper >= upper_base,
                    "height={height}: mirrored row {y_upper} is not in the upper half"
                );
                assert_eq!(
                    lower_base - y_lower,
                    y_upper - upper_base,
                    "height={height}: offsets differ for y_lower={y_lower}"
                );
            }

            // The bottom row must always be reachable by the lower half.
            assert!(lower_base <= height - 1);
        }
    }

    /// Bars must never leave the axis: an overflowing `bars`/`bar_width`
    /// request (e.g. 200 bars in a 100-column terminal, or a 61-column bar in
    /// a 25-row terminal) used to wrap onto the next line and visually
    /// "overlap".
    #[test]
    fn layout_never_overflows_the_axis() {
        for dim in [1usize, 7, 24, 25, 80, 100, 239] {
            for w in [1usize, 2, 3, 8, 64] {
                for sp in [0usize, 1, 4, 16] {
                    for n in [1usize, 2, 3, 17, 33, 200, 512] {
                        let l = BarLayout::new(dim, n, w, sp);
                        let last = l.starts[l.len() - 1];
                        assert!(
                            last + l.bar_width <= dim,
                            "bar {last}+{} exceeds axis {dim} (n={n} w={w} sp={sp})",
                            l.bar_width
                        );
                        // No two bars may share a cell.
                        let occupied: Vec<usize> = l
                            .rev
                            .iter()
                            .enumerate()
                            .filter_map(|(c, v)| v.map(|_| c))
                            .collect();
                        assert_eq!(occupied.len(), l.len() * l.bar_width, "cell reuse");
                    }
                }
            }
        }
    }

    /// Every inter-bar gap is exactly the configured spacing; the leftover
    /// cells become symmetric axis-edge margins, so the block no longer
    /// wobbles on resize. With `spacing == 0` the gaps are zero-width and
    /// spare cells stay at the edges instead (see
    /// [`autofit_spacing_zero_never_holes_the_block`]).
    #[test]
    fn autofit_layout_keeps_uniform_gaps_with_edge_margins() {
        for dim in [80usize, 100, 121, 239] {
            for w in [1usize, 2, 3] {
                for sp in [0usize, 1, 2] {
                    for n in [5usize, 17, 33] {
                        let l = BarLayout::new(dim, n, w, sp);
                        let gaps: Vec<usize> = (0..l.len() - 1)
                            .map(|i| l.gap_before(i + 1))
                            .collect();
                        assert!(
                            gaps.iter().all(|&g| g == sp),
                            "gaps must all equal spacing (n={n} w={w} sp={sp} dim={dim}): {gaps:?}"
                        );
                        let used = l.starts[l.len() - 1] + l.bar_width;
                        let (left, right) = (l.starts[0], dim.saturating_sub(used));
                        let n2 = n.min(((dim + sp) / (w + sp)).max(1));
                        let leftover = dim - (n2 * w + (n2 - 1) * sp);
                        assert!(
                            used <= dim && left + right == leftover && right.abs_diff(left) <= 1,
                            "spare cells must split symmetrically around the block \
                             (n={n} w={w} sp={sp} dim={dim}): used={used} left={left} right={right}"
                        );
                    }
                }
            }
        }
    }

    /// The permanently missing bar: a left/right stereo layout on an odd
    /// terminal height (24 even bars on 25 rows) used to fold the single
    /// spare cell into the bar block as a blank row mid-cover (row 23 of 25
    /// read empty every frame). The spare cell must become edge padding, and
    /// every bar row must stay mapped.
    #[test]
    fn autofit_spacing_zero_never_holes_the_block() {
        for (dim, n) in [(25usize, 24usize), (13, 12), (24, 24)] {
            let l = BarLayout::new(dim, n, 1, 0);
            assert_eq!(l.len(), n, "dim={dim}");
            let spare: Vec<usize> = l
                .rev
                .iter()
                .enumerate()
                .filter_map(|(c, v)| if v.is_none() { Some(c) } else { None })
                .collect();
            assert_eq!(
                spare.len(),
                dim - n,
                "dim={dim} n={n}: expected {spare:?} spare cells"
            );
            for cell in spare {
                assert!(
                    cell == 0 || cell == dim - 1,
                    "dim={dim}: spare cell {cell} is inside the bar block"
                );
            }
            assert_eq!(l.rev.iter().flatten().count(), n, "every bar mapped");
        }
    }

    /// Fixed counts keep the configured spacing and the block is always centred.
    #[test]
    fn fixed_layout_is_centred() {
        let l = BarLayout::new(100, 10, 2, 1);
        assert_eq!(l.starts[0], 35); // (100 - (10*2 + 9*1)) / 2 = 35
        assert_eq!(l.gap_before(3), 1);
    }

    /// Regression guard for the "ragged left half" layout: auto-fit used to
    /// add every leftover cell to the *first* gaps, so 32 two-column bars on a
    /// 100-column terminal rendered `[2,2,2,2,2,1,1,…]`. Every gap must be
    /// exactly the configured spacing and all spare cells must sit in the two
    /// symmetric edge margins.
    #[test]
    fn autofit_layout_keeps_spacing_uniform() {
        let l = BarLayout::new(100, 32, 2, 1);
        let gaps: Vec<usize> = (0..l.len() - 1).map(|i| l.gap_before(i + 1)).collect();
        assert!(
            gaps.iter().all(|&g| g == 1),
            "all gaps must be exactly 1: {gaps:?}"
        );

        // 100 cols / 32 bars: block = 32*2 + 31 = 95, leftover 5 -> centred
        // margins of 2 left / 3 right. Nothing may eat into the gap list.
        assert_eq!(l.starts[0], 2);
        assert_eq!(l.starts[l.len() - 1] + l.bar_width, 97);
    }

    /// A flat trace must draw as one thin band on a single half-row, not a
    /// filled slab.
    #[test]
    fn waveform_flat_trace_is_a_single_thin_line() {
        let trace = vec![0.7; 400];
        let spans = waveform_cells(&trace, 50, 39);
        assert_eq!(spans.len(), 50);
        let h: usize = (0.3_f64 * 40.0_f64).round() as usize;
        for &(lo, hi) in &spans {
            assert_eq!(
                (lo, hi),
                (h, h),
                "flat trace widened into a filled band"
            );
        }
    }

    /// A slow dense sine stays a thin joined line; the old min/max envelope
    /// filled each column with the whole slice band.
    #[test]
    fn waveform_dense_sine_stays_a_thin_line() {
        let n = 5000;
        let trace: Vec<f64> = (0..n)
            .map(|i| 0.5 + 0.5 * (i as f64 * 0.0005).sin())
            .collect();
        let spans = waveform_cells(&trace, 50, 39);
        assert_eq!(spans.len(), 50);
        let max_span = spans.iter().map(|(lo, hi)| hi - lo).max().unwrap_or(0);
        assert!(
            max_span <= 3,
            "dense sine degenerated into a {max_span}-halfrow slab"
        );
    }

    /// Upsampling (fewer trace points than columns) interpolates smoothly.
    #[test]
    fn waveform_upsample_interpolates() {
        let trace = vec![0.0, 1.0];
        let spans = waveform_cells(&trace, 5, 9);
        assert_eq!(spans.len(), 5);
        // Endpoints map to the trace values pinned to the extreme half-rows:
        // 0.0 → bottom half-row 9, 1.0 → top 0 (the last span is joined with
        // its previous column's band, so it covers 0..=2 by design).
        assert_eq!(spans[0], (9, 9));
        assert_eq!(spans[4], (0, 2));
    }

    /// The dynamics envelope is a local min/max band, wider than the line but
    /// never physically smaller than the trace it wraps.
    #[test]
    fn envelope_band_wraps_the_thin_line() {
        let trace = vec![0.1, 0.9, 0.1, 0.9];
        let half_max = 19;
        let line = waveform_cells(&trace, 4, half_max);
        let env = envelope_cells(&trace, 4, half_max, 1);
        for (i, &(e_lo, e_hi)) in env.iter().enumerate() {
            assert!(e_lo <= line[i].0, "env top above trace at col {i}");
            assert!(e_hi >= line[i].1, "env bottom below trace at col {i}");
        }
        // A point flanked by both extremes catches both in its radius-1 window.
        // 0.9 → top half-row 2, 0.1 → bottom 17.
        assert_eq!(env[1], (2, 17));
        assert_eq!(env[2], (2, 17));
    }
}

pub mod color;
pub mod raw;
pub mod terminal;

use crate::config::{Config, OutputMethod};
use self::raw::RawRenderer;
use self::terminal::TerminalRenderer;
use std::io;

/// Smallest number of bars the auto-fit layout will ever be reduced to by a
/// wide `bar_width`.
///
/// A `bar_width = 64` on a 100-column terminal is a single 64-column wall, not
/// a spectrum — and because the width is persisted by the Left/Right keys, one
/// over-eager press used to leave the visualizer looking permanently broken
/// after a restart. Clamping the *effective* width keeps the configured value
/// intact (so shrinking the config back down still works) while guaranteeing a
/// readable spectrum on screen.
pub const MIN_BARS_ON_AXIS: usize = 4;

/// Bars that fit along an axis of `dim` cells with the given bar width and
/// spacing. Never returns 0.
pub fn bars_that_fit(dim: usize, bar_width: usize, bar_spacing: usize) -> usize {
    let dim = dim.max(1);
    let w = bar_width.max(1).min(dim);
    let sp = bar_spacing.min(dim);
    ((dim + sp) / (w + sp)).max(1)
}

/// Widest bar (capped at 64) that still leaves at least [`MIN_BARS_ON_AXIS`]
/// bars on an axis of `dim` cells — the "fewest bars" end of the auto-mode
/// range.
pub fn widest_sane_bar_width(dim: usize, bar_spacing: usize) -> usize {
    let mut w = 1;
    while w < 64 && bars_that_fit(dim, w + 1, bar_spacing) >= MIN_BARS_ON_AXIS {
        w += 1;
    }
    w
}

/// The bar width actually used for layout and rendering.
///
/// In auto mode (`bars = 0`) this is the configured width clamped so at least
/// [`MIN_BARS_ON_AXIS`] bars still fit the axis. A fixed bar count is left
/// alone — the layout already clamps the *count* to what fits.
pub fn effective_bar_width(config: &Config, dim: usize) -> usize {
    let w = config.general.bar_width.max(1);
    if config.general.bars == 0 {
        w.min(widest_sane_bar_width(dim, config.general.bar_spacing))
    } else {
        w
    }
}

pub enum Renderer {
    Terminal(TerminalRenderer),
    Raw(RawRenderer),
}

impl Renderer {
    pub fn new(config: &Config) -> Result<Self, String> {
        match config.output.method {
            OutputMethod::Noncurses | OutputMethod::Ncurses => {
                let term = TerminalRenderer::new(config)
                    .map_err(|e| format!("Failed to initialize terminal renderer: {e}"))?;
                Ok(Renderer::Terminal(term))
            }
            OutputMethod::Raw => {
                let raw = RawRenderer::new(&config.output)
                    .map_err(|e| format!("Failed to initialize raw renderer: {e}"))?;
                Ok(Renderer::Raw(raw))
            }
        }
    }

    pub fn render(
        &mut self,
        bars: &[f64],
        config: &Config,
        freqs: Option<&[f64]>,
    ) -> io::Result<()> {
        match self {
            Renderer::Terminal(t) => t.render(bars, config, freqs),
            Renderer::Raw(r) => r.render(bars),
        }
    }

    pub fn resize(&mut self, config: &Config, width: u16, height: u16, num_bars: usize) {
        if let Renderer::Terminal(t) = self {
            t.resize(config, width, height, num_bars);
        }
    }

    pub fn reload_colors(&mut self, config: &Config, num_bars: usize) {
        if let Renderer::Terminal(t) = self {
            t.reload_colors(config, num_bars);
        }
    }

    /// Show a transient status message (terminal output only; a no-op for the
    /// raw data stream).
    pub fn set_status(&mut self, msg: impl Into<String>) {
        if let Renderer::Terminal(t) = self {
            t.set_status(msg);
        }
    }

    /// Current terminal size in cells (terminal output only; the raw stream
    /// has no notion of a screen).
    pub fn dims(&self) -> (u16, u16) {
        match self {
            Renderer::Terminal(t) => t.dims(),
            Renderer::Raw(_) => (0, 0),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    /// Regression guard for the "single full-screen wall" layout bug: with
    /// `bars = 0` the bar width is persisted by the arrow keys, so one
    /// over-eager press used to leave `bar_width = 64` on disk. On a 100-column
    /// terminal that auto-fit to exactly one 64-column bar and the visualizer
    /// looked permanently broken after a restart.
    #[test]
    fn auto_mode_never_renders_a_single_wall() {
        let mut config = Config::default();
        config.general.bars = 0;
        config.general.bar_width = 64;
        config.general.bar_spacing = 1;
        for dim in [40usize, 80, 100, 200, 400] {
            let w = effective_bar_width(&config, dim);
            let n = bars_that_fit(dim, w, config.general.bar_spacing);
            assert!(
                n >= MIN_BARS_ON_AXIS,
                "dim={dim}: bar_width 64 auto-fit to {n} bars (w={w})"
            );
        }
    }

    /// A *fixed* bar count is deliberately left alone: the layout clamps the
    /// count to what fits, and the user asked for a specific width.
    #[test]
    fn fixed_mode_keeps_the_configured_width() {
        let mut config = Config::default();
        config.general.bars = 12;
        config.general.bar_width = 40;
        assert_eq!(effective_bar_width(&config, 100), 40);
    }

    #[test]
    fn widest_sane_width_keeps_at_least_min_bars_when_possible() {
        for dim in [9usize, 12, 25, 80, 100, 240] {
            for sp in [0usize, 1, 2, 5] {
                let w = widest_sane_bar_width(dim, sp);
                if bars_that_fit(dim, 1, sp) >= MIN_BARS_ON_AXIS {
                    let n = bars_that_fit(dim, w, sp);
                    assert!(
                        n >= MIN_BARS_ON_AXIS,
                        "dim={dim} sp={sp}: widest sane width {w} leaves only {n} bars"
                    );
                } else {
                    // A degenerate axis (tiny terminal, huge spacing) cannot
                    // reach the minimum at any width; width 1 is the best case.
                    assert_eq!(w, 1, "dim={dim} sp={sp}: width must fall back to 1");
                }
            }
        }
    }
}

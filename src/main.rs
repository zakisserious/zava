
use std::env;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use zava::audio::AudioSource;
use zava::config::{Config, InputMethod, MonoOption, Orientation, OutputMethod, TerminalColor};
use zava::dsp::filter::{apply_monstercat_filter, interpolate_user_eq};
use zava::dsp::CavaPlan;
use zava::render::Renderer;
use zava::terminal_guard::{ActionEvent, TerminalGuard};

/// One-line summary of the current bar layout, shown after Left/Right presses
/// so the effect of a keypress is visible even when the count doesn't move
/// (e.g. already at the auto-fit ceiling). Shows the *effective* width so what
/// the overlay says always matches what the renderer draws.
fn bars_status(num_bars: usize, config: &Config) -> String {
    let mode = if config.general.bars == 0 { "auto" } else { "fixed" };
    let axis = term_axis(config);
    let width = if config.general.bars == 0 {
        zava::render::effective_bar_width(config, axis)
    } else {
        config.general.bar_width
    };
    format!(
        "bars: {} ({})  w={}  sp={}",
        num_bars, mode, width, config.general.bar_spacing
    )
}

fn print_help() {
    println!(
        "zava - Console-based Audio Visualizer in Rust\n\nUsage: zava [options]\n\nOptions:\n    -p, --config <path>    Path to config file\n    -v, --version          Print version\n    -h, --help             Print help\n    --test                 Use synthetic test audio signal\n    --generate-config      Write a config file and exit\n    --force                With --generate-config, overwrite an existing file\n    --method <method>      Audio input method (pipewire, pulse, alsa, fifo, cpal, test)\n    --source <source>      Audio device source\n    --output <method>      Output method (noncurses, ncurses, raw)\n    --channels <mode>      stereo or mono\n\nControls:\n    Up        Increase sensitivity\n    Down      Decrease sensitivity\n    Left      Decrease number of bars\n    Right     Increase number of bars\n    m         Open settings menu\n    o         Cycle orientation\n    r         Reload configuration\n    c         Reload colors only\n    f         Cycle foreground color\n    b         Cycle background color\n    q         Quit\n\nConfig:\n    ~/.config/zava/config (or $XDG_CONFIG_HOME/zava/config).\n    Zava never reads ~/.config/cava/config.\n    Reference: /usr/share/zava/config.example\n"
    );
}

fn print_version() {
    println!("zava 0.1.0 (Rust CAVA remake)");
}

/// Maximum bar count that fits an axis of `dim` cells without any bar
/// overflowing the terminal (overflow used to wrap onto the next line and
/// look like overlapping bars).
fn max_bars_for_axis(dim: usize, bar_width: usize, bar_spacing: usize) -> usize {
    zava::render::bars_that_fit(dim, bar_width, bar_spacing)
}

/// The terminal dimension that bars are laid out along: rows for the vertical
/// (left/right) orientations, columns for everything else.
fn term_axis(config: &Config) -> usize {
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    match config.output.orientation {
        Orientation::Left | Orientation::Right => height as usize,
        _ => width as usize,
    }
}

/// Auto mode's bar-width range is 1..=64, but a wide bar is a single monolith
/// that takes many keypresses to walk back from. When wrapping, jump to the
/// widest bar that still leaves a usable number of bars on screen instead.
fn wrap_bar_width(dim: usize, spacing: usize) -> usize {
    zava::render::widest_sane_bar_width(dim, spacing)
}

/// Number of bars to render: in auto mode (`bars = 0`) the count that fills
/// the terminal axis, otherwise the configured count capped to what fits.
fn calculate_bars_count(config: &Config, orientation: Orientation, output_channels: u32) -> usize {
    let (width, height) = crossterm::terminal::size().unwrap_or((80, 24));
    let (width, height) = (width as usize, height as usize);
    let dim = match orientation {
        Orientation::Left | Orientation::Right => height,
        _ => width,
    };
    // Use the *effective* width so the count always matches what the renderer
    // actually draws: in auto mode a too-wide `bar_width` is clamped there, and
    // a mismatch would leave the plan cut for a different number of bands than
    // are on screen.
    let bar_width = zava::render::effective_bar_width(config, dim);
    let mut fit = max_bars_for_axis(dim, bar_width, config.general.bar_spacing);
    // Stereo mirrors the two channel halves, so keep the count even.
    if output_channels == 2 {
        fit -= fit % 2;
    }
    if config.general.bars > 0 {
        config.general.bars.min(fit).max(1)
    } else {
        fit
    }
}

fn recompute_bars_and_plan(
    config: &Config,
    audio_source: &AudioSource,
    dsp_plan: &mut CavaPlan,
    renderer: &mut Renderer,
) -> (usize, usize) {
    let mut num_bars = calculate_bars_count(config, config.output.orientation, config.output.channels);
    let output_channels = config.output.channels as usize;
    let audio_channels = audio_source.channels() as usize;

    let bars_per_audio_chan = if audio_channels == 2 && output_channels == 2 {
        (num_bars / 2).max(1)
    } else {
        num_bars.max(1)
    };
    num_bars = if audio_channels == 2 && output_channels == 2 {
        bars_per_audio_chan * 2
    } else {
        bars_per_audio_chan
    };

    if let Ok(new_plan) = CavaPlan::new(
        bars_per_audio_chan,
        audio_source.sample_rate(),
        audio_channels,
        config.general.autosens,
        config.smoothing.noise_reduction,
        config.general.lower_cutoff_freq,
        config.general.higher_cutoff_freq,
        config.general.scaling,
        config.smoothing.gravity,
    ) {
        *dsp_plan = new_plan;
    }

    if let Ok((w, h)) = crossterm::terminal::size() {
        renderer.resize(config, w, h, num_bars);
    }

    // Clear terminal screen to prevent ghost bars and artifacts across orientations
    let mut stdout = std::io::stdout();
    let _ = write!(stdout, "\x1b[2J\x1b[H");
    let _ = stdout.flush();

    (num_bars, bars_per_audio_chan)
}

/// Start index of the oscilloscope display window inside `ring`.
///
/// Prefers the **latest rising crossing** that still leaves `window` samples
/// available — the same trick hardware scopes use to freeze a repeating
/// waveform in place. Anchoring every frame at the same phase is what turns
/// the sample stream into a readable, non-jittering trace.
///
/// `hyst` is the trigger hysteresis: a rising zero crossing is only accepted
/// after the signal has dipped to at least `-hyst`, so shallow noise wiggles
/// around zero cannot capture the trigger and make the trace jump phase. Pass
/// `0.0` for a plain zero-crossing trigger.
///
/// Falls back to the newest possible window (`ring.len() - window`) when no
/// crossing exists (silence, DC offset, or a flat positive signal), so the
/// scope still shows *something* sensible instead of nothing.
///
/// Returns `None` only when `ring` is shorter than `window`.
pub fn wave_trigger_start(ring: &[f64], window: usize, hyst: f64) -> Option<usize> {
    if ring.len() < window {
        return None;
    }
    let last_start = ring.len() - window;
    let mut armed = false;
    let mut last = None;
    for i in 1..=last_start {
        if ring[i] <= -hyst {
            armed = true;
        }
        if armed && ring[i - 1] <= 0.0 && ring[i] > 0.0 {
            last = Some(i);
            armed = false;
        }
    }
    last.or(Some(last_start))
}

/// One-pole low-pass coefficient for cutoff `fc` at sample rate `fs`.
pub fn lowpass_coeff(fc: f64, fs: f64) -> f64 {
    1.0 - (-2.0 * std::f64::consts::PI * fc / fs).exp()
}

/// Auto-gain for the oscilloscope trace.
///
/// Takes the current window peak and the running peak/gain, returns the new
/// `(peak, gain)`. The peak snaps up to a loud transient but releases slowly
/// (`0.995`/frame) so the trace does not breathe, and the gain itself is
/// eased toward its target so amplitude changes never step the whole trace.
pub fn auto_gain(peak: f64, prev_peak: f64, prev_gain: f64) -> (f64, f64) {
    let peak = if peak > prev_peak {
        peak
    } else {
        prev_peak * 0.995
    };
    let target = if peak > 1e-6 {
        (0.85 / peak).min(200.0)
    } else {
        0.0
    };
    let gain = prev_gain + (target - prev_gain) * 0.2;
    (peak, gain)
}

/// Decimate a triggered window down onto `width` columns. Each output column
/// is the mean of its evenly sized slice of the window (box averaging): it
/// anti-aliases high-frequency content instead of letting a nearest/lerp
/// take turn music into a static-ish noise trace. When there are fewer
/// samples than columns it interpolates so no column is ever empty.
pub fn downsample_box_average(window: &[f64], width: usize) -> Vec<f64> {
    let len = window.len();
    if width == 0 || len == 0 {
        return Vec::new();
    }
    if width >= len {
        let last = len - 1;
        let step = last as f64 / (width - 1).max(1) as f64;
        return (0..width)
            .map(|n| {
                let pos = n as f64 * step;
                let i0 = (pos as usize).min(last);
                let i1 = (i0 + 1).min(last);
                let frac = pos - i0 as f64;
                window[i0] + (window[i1] - window[i0]) * frac
            })
            .collect();
    }
    (0..width)
        .map(|n| {
            let i0 = n * len / width;
            let i1 = ((n + 1) * len / width).max(i0 + 1).min(len);
            let slice = &window[i0..i1];
            slice.iter().sum::<f64>() / slice.len() as f64
        })
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    let mut config_path: Option<PathBuf> = None;
    let mut force_test = false;
    let mut generate_config = false;
    let mut force_overwrite = false;
    let mut override_method: Option<InputMethod> = None;
    let mut override_source: Option<String> = None;
    let mut override_output: Option<OutputMethod> = None;
    let mut override_channels: Option<u32> = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--version" => {
                print_version();
                return Ok(());
            }
            "-h" | "--help" => {
                print_help();
                return Ok(());
            }
            "-p" | "--config" => {
                if i + 1 < args.len() {
                    config_path = Some(PathBuf::from(&args[i + 1]));
                    i += 1;
                }
            }
            "--test" => {
                force_test = true;
            }
            "--generate-config" | "--gen-config" => {
                generate_config = true;
            }
            "--force" => {
                force_overwrite = true;
            }
            "--method" => {
                if i + 1 < args.len() {
                    let m = match args[i + 1].to_lowercase().as_str() {
                        "pipewire" => InputMethod::Pipewire,
                        "pulse" => InputMethod::Pulse,
                        "alsa" => InputMethod::Alsa,
                        "fifo" => InputMethod::Fifo,
                        "cpal" => InputMethod::Cpal,
                        "test" => InputMethod::Test,
                        _ => InputMethod::Pipewire,
                    };
                    override_method = Some(m);
                    i += 1;
                }
            }
            "--source" => {
                if i + 1 < args.len() {
                    override_source = Some(args[i + 1].clone());
                    i += 1;
                }
            }
            "--output" => {
                if i + 1 < args.len() {
                    let o = match args[i + 1].to_lowercase().as_str() {
                        "noncurses" => OutputMethod::Noncurses,
                        "ncurses" => OutputMethod::Ncurses,
                        "raw" => OutputMethod::Raw,
                        _ => OutputMethod::Noncurses,
                    };
                    override_output = Some(o);
                    i += 1;
                }
            }
            "--channels" => {
                if i + 1 < args.len() {
                    if args[i + 1].eq_ignore_ascii_case("mono") || args[i + 1] == "1" {
                        override_channels = Some(1);
                    } else {
                        override_channels = Some(2);
                    }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }

    let mut config = Config::load_or_default(config_path.as_deref());
    // Zava never reads `~/.config/cava/config` (its serialiser would rewrite the
    // user's CAVA setup and CAVA's fixed bar counts would disable auto-fit), so
    // say so explicitly when a legacy file is sitting there unused.
    if let Some(legacy) = Config::locate_legacy_cava_path(config_path.as_deref()) {
        eprintln!(
            "Warning: ignoring legacy CAVA config {}. Zava is config-isolated; \
             use {} (created on first save).",
            legacy.display(),
            config
                .config_path
                .as_deref()
                .unwrap_or_else(|| Path::new("~/.config/zava/config"))
                .display(),
        );
    }
    if generate_config {
        // Write a config to the standard location (or to -p PATH) and exit, so a
        // fresh install can get a usable file without opening the menu.
        // An existing file is never overwritten unless --force is given:
        // `save()` rewrites every key it owns, which would silently reset the
        // user's values back to the defaults.
        let target = config_path
            .clone()
            .or_else(|| config.config_path.clone())
            .or_else(|| Config::locate_config_path(None));
        let resolved = target
            .clone()
            .unwrap_or_else(|| PathBuf::from("~/.config/zava/config"));
        if resolved.exists() && !force_overwrite {
            eprintln!(
                "Refusing to overwrite existing config {}.\n\
                 Pass --force to reset its values to the defaults, or -p <path> to write elsewhere.\n\
                 The fully commented reference is /usr/share/zava/config.example.",
                resolved.display()
            );
            std::process::exit(1);
        }
        let mut fresh = Config::default();
        fresh.config_path = target;
        if let Some(p) = &fresh.config_path {
            if let Some(parent) = p.parent() {
                std::fs::create_dir_all(parent)?;
            }
        }
        fresh.save()?;
        println!("Wrote config to {}", resolved.display());
        println!("Reference with all documented options: /usr/share/zava/config.example");
        return Ok(());
    }

    if force_test {
        config.input.method = InputMethod::Test;
    }
    if let Some(m) = override_method {
        config.input.method = m;
    }
    if let Some(s) = override_source {
        config.input.source = s;
    }
    if let Some(o) = override_output {
        config.output.method = o;
    }
    if let Some(c) = override_channels {
        config.output.channels = c;
    }

    // A `raw` output method writes a binary data stream to stdout. On a
    // terminal that is unreadable garbage — and because the settings menu
    // saves on every adjustment, one accidental Right-press on the Output
    // Method row used to brick the display on the next launch. Fall back to
    // the ANSI renderer whenever stdout is a TTY.
    if config.output.method == OutputMethod::Raw && std::io::stdout().is_terminal() {
        eprintln!(
            "warning: output method is `raw` but stdout is a terminal; \
             using `noncurses` instead (raw mode is for piping binary data)"
        );
        config.output.method = OutputMethod::Noncurses;
    }

    let is_terminal = config.output.method != OutputMethod::Raw;
    let mut term_guard = if is_terminal {
        let use_alternate = config.output.method == OutputMethod::Ncurses;
        Some(TerminalGuard::new(use_alternate)?)
    } else {
        None
    };

    let mut audio_source = AudioSource::new(&config)
        .map_err(|e| format!("Failed to initialize audio source: {e}"))?;

    let effective_rate = audio_source.sample_rate();
    let audio_channels = audio_source.channels() as usize;

    let mut num_bars = calculate_bars_count(&config, config.output.orientation, config.output.channels);
    let output_channels = config.output.channels as usize;

    let mut bars_per_audio_chan = if audio_channels == 2 && output_channels == 2 {
        (num_bars / 2).max(1)
    } else {
        num_bars.max(1)
    };
    num_bars = if audio_channels == 2 && output_channels == 2 {
        bars_per_audio_chan * 2
    } else {
        bars_per_audio_chan
    };

    let mut dsp_plan = CavaPlan::new(
        bars_per_audio_chan,
        effective_rate,
        audio_channels,
        config.general.autosens,
        config.smoothing.noise_reduction,
        config.general.lower_cutoff_freq,
        config.general.higher_cutoff_freq,
        config.general.scaling,
        config.smoothing.gravity,
    )
    .map_err(|e| format!("Failed to initialize DSP plan: {e}"))?;

    let mut user_sensitivity = (config.general.sensitivity / 100.0).max(0.01);

    let mut renderer = Renderer::new(&config)
        .map_err(|e| format!("Failed to initialize renderer: {e}"))?;

    let mut frame_interval = Duration::from_nanos((1_000_000_000.0 / config.general.framerate as f64) as u64);
    let mut last_config_check = Instant::now();
    let mut last_config_mtime = config.config_path.as_ref().and_then(|p| std::fs::metadata(p).ok()?.modified().ok());

    let mut silent_seconds = 0.0;
    let mut prev_frame_time = Instant::now();
    // Running peak of the waveform trace, used to auto-gain the oscilloscope
    // so a quiet signal still sweeps most of the screen height.
    let mut wave_peak = 0.0f64;
    // Eased auto-gain and the two cascaded one-pole low-pass states that shape
    // the oscilloscope input (see the waveform branch below).
    let mut wave_gain = 0.0f64;
    let mut wave_lp1 = 0.0f64;
    let mut wave_lp2 = 0.0f64;
    // Rolling history of mono samples for the oscilloscope. A single 512-sample
    // frame (43 ms) is too short for a useful, phase-stable time base, so the
    // last ~186 ms are kept and each frame draws a fixed-length window anchored
    // at a rising zero crossing (oscilloscope "trigger"). Without the trigger
    // every frame shows an arbitrary slice and even a steady tone jitters.
    let mut wave_ring: Vec<f64> = Vec::new();

    loop {
        let loop_start = Instant::now();
        let dt = loop_start.duration_since(prev_frame_time).as_secs_f64();
        prev_frame_time = loop_start;

        // Process terminal events
        while let Some(event) = term_guard.as_ref().and_then(|g| g.poll_event()) {
                match event {
                    ActionEvent::Quit => {
                        return Ok(());
                    }
                    ActionEvent::IncreaseSensitivity => {
                        user_sensitivity = (user_sensitivity * 1.15).min(100.0);
                        renderer.set_status(format!(
                            "sensitivity: {:.0}%",
                            user_sensitivity * 100.0
                        ));
                    }
                    ActionEvent::DecreaseSensitivity => {
                        user_sensitivity = (user_sensitivity * 0.85).max(0.01);
                        renderer.set_status(format!(
                            "sensitivity: {:.0}%",
                            user_sensitivity * 100.0
                        ));
                    }
                    ActionEvent::DecreaseBars => {
                        // Left arrow: fewer bars (wider bars in auto mode).
                        // Bounded by default; `wrap_bars` jumps to the other end
                        // of the range instead of stopping.
                        let axis = term_axis(&config);
                        let max_w = wrap_bar_width(axis, config.general.bar_spacing);
                        if config.general.bars > 0 {
                            if config.general.bars <= 1 && config.general.wrap_bars {
                                config.general.bars = max_bars_for_axis(
                                    axis,
                                    zava::render::effective_bar_width(&config, axis),
                                    config.general.bar_spacing,
                                );
                            } else {
                                config.general.bars = config.general.bars.saturating_sub(1).max(1);
                            }
                        } else {
                            // Auto mode: widen the bars, which means fewer of
                            // them. Widths are clamped to the widest value that
                            // still leaves a readable spectrum (a legacy
                            // `bar_width = 64` is pulled back in here too).
                            let current = zava::render::effective_bar_width(&config, axis);
                            config.general.bar_width = if current >= max_w {
                                if config.general.wrap_bars {
                                    1
                                } else {
                                    max_w
                                }
                            } else {
                                current + 1
                            };
                        }
                        let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                        num_bars = nb;
                        bars_per_audio_chan = bpc;
                        // Keep the stored count truthful: it is the clamped
                        // count that actually fits the terminal.
                        if config.general.bars > 0 {
                            config.general.bars = nb;
                        }
                        renderer.set_status(bars_status(nb, &config));
                    }
                    ActionEvent::IncreaseBars => {
                        // Right arrow: more bars (narrower bars in auto mode).
                        // The count is hard-capped by what actually fits the
                        // terminal in `calculate_bars_count`, so holding the key
                        // can never overflow the width and make bars wrap.
                        let axis = term_axis(&config);
                        let max_w = wrap_bar_width(axis, config.general.bar_spacing);
                        let cap = max_bars_for_axis(
                            axis,
                            zava::render::effective_bar_width(&config, axis),
                            config.general.bar_spacing,
                        );
                        if config.general.bars > 0 {
                            if config.general.bars >= cap && config.general.wrap_bars {
                                config.general.bars = 1;
                            } else {
                                config.general.bars = (config.general.bars + 1).min(cap);
                            }
                        } else {
                            // Auto mode: narrow the bars, which means more of
                            // them. At width 1 the spectrum is already as dense
                            // as the terminal allows, so only wrap when asked.
                            let current = zava::render::effective_bar_width(&config, axis);
                            if current <= 1 {
                                if config.general.wrap_bars {
                                    config.general.bar_width = max_w;
                                }
                            } else {
                                config.general.bar_width = current - 1;
                            }
                        }
                        let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                        num_bars = nb;
                        bars_per_audio_chan = bpc;
                        if config.general.bars > 0 {
                            config.general.bars = nb;
                        }
                        renderer.set_status(bars_status(nb, &config));
                    }
                    ActionEvent::CycleOrientation => {
                        config.output.orientation = config.output.orientation.cycle(config.output.method);
                        let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                        num_bars = nb;
                        bars_per_audio_chan = bpc;
                        renderer.set_status(format!(
                            "orientation: {}  bars: {}",
                            config.output.orientation.as_str(),
                            nb
                        ));
                    }
                    ActionEvent::CycleForegroundColor => {
                        let colors = [
                            TerminalColor::Default,
                            TerminalColor::Red,
                            TerminalColor::Green,
                            TerminalColor::Yellow,
                            TerminalColor::Blue,
                            TerminalColor::Magenta,
                            TerminalColor::Cyan,
                            TerminalColor::White,
                        ];
                        let current_idx = colors.iter().position(|c| *c == config.color.foreground).unwrap_or(0);
                        config.color.foreground = colors[(current_idx + 1) % colors.len()].clone();
                        renderer.reload_colors(&config, num_bars);
                    }
                    ActionEvent::CycleBackgroundColor => {
                        let colors = [
                            TerminalColor::Default,
                            TerminalColor::Black,
                            TerminalColor::Blue,
                            TerminalColor::Cyan,
                            TerminalColor::White,
                        ];
                        let current_idx = colors.iter().position(|c| *c == config.color.background).unwrap_or(0);
                        config.color.background = colors[(current_idx + 1) % colors.len()].clone();
                        renderer.reload_colors(&config, num_bars);
                    }
                    ActionEvent::ReloadColors => {
                        let _ = config.reload_colors_only();
                        renderer.reload_colors(&config, num_bars);
                    }
                    ActionEvent::ReloadConfig => {
                        let _ = config.reload();
                        user_sensitivity = (config.general.sensitivity / 100.0).max(0.01);
                        let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                        num_bars = nb;
                        bars_per_audio_chan = bpc;
                        renderer.reload_colors(&config, num_bars);
                    }
                    ActionEvent::Resize => {
                        let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                        num_bars = nb;
                        bars_per_audio_chan = bpc;
                    }
                    ActionEvent::OpenMenu => {
                        if let Some(guard) = &mut term_guard {
                            guard.suspend();
                            let _ = zava::menu::run_menu(&mut config);
                            let _ = guard.resume();
                            let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                            num_bars = nb;
                            bars_per_audio_chan = bpc;
                            renderer.reload_colors(&config, num_bars);
                            frame_interval = Duration::from_secs_f64(1.0 / config.general.framerate.max(1) as f64);
                        }
                }
            }
        }
        // Live config file reload
        if config.general.live_config && last_config_check.elapsed() > Duration::from_millis(500) {
            last_config_check = Instant::now();
            if let Some(p) = &config.config_path {
                if let Ok(meta) = std::fs::metadata(p) {
                    if let Ok(mtime) = meta.modified() {
                        if Some(mtime) != last_config_mtime {
                            last_config_mtime = Some(mtime);
                            let _ = config.reload();
                            user_sensitivity = (config.general.sensitivity / 100.0).max(0.01);
                            let (nb, bpc) = recompute_bars_and_plan(&config, &audio_source, &mut dsp_plan, &mut renderer);
                            num_bars = nb;
                            bars_per_audio_chan = bpc;
                            renderer.reload_colors(&config, num_bars);
                        }
                    }
                }
            }
        }

        // Read audio samples
        let samples = audio_source.read_samples().unwrap_or_default();
        let is_silent = samples.is_empty() || samples.iter().all(|&s| s == 0.0);

        if is_silent {
            silent_seconds += dt;
        } else {
            silent_seconds = 0.0;
        }

        // Sleep mode when silence exceeds sleep_timer
        if config.general.sleep_timer > 0 && silent_seconds > config.general.sleep_timer as f64 {
            thread::sleep(Duration::from_millis(200));
            continue;
        }

        let user_eq = interpolate_user_eq(&config.eq, bars_per_audio_chan);

                // Waveform mode or Spectrum FFT mode
        let final_bars = if config.output.waveform {
            // Oscilloscope mode: the raw mono mix is low-passed (cutoff from
            // Noise Reduction), stored in a rolling ring, and each frame draws
            // a fixed-length window anchored at a hysteresis trigger crossing,
            // so a repeating wave stays frozen instead of jittering.
            const PCM_SCALE: f64 = 1.0 / 32768.0;
            // The test-signal backend already emits normalised floats? No —
            // every backend emits integer-PCM-scaled samples, including the
            // test signal, so one scale factor fits all.
            let scale = |s: f64| -> f64 { s * PCM_SCALE };

            // Mix down to mono honouring the configured mono option.
            let mono: Vec<f64> = if audio_channels == 2 {
                samples
                    .chunks(2)
                    .map(|c| match config.output.mono_option {
                        MonoOption::Left => scale(c[0]),
                        MonoOption::Right => scale(c.get(1).copied().unwrap_or(c[0])),
                        MonoOption::Average => {
                            (scale(c[0]) + scale(c.get(1).copied().unwrap_or(0.0))) * 0.5
                        }
                    })
                    .collect()
            } else {
                samples.iter().copied().map(scale).collect()
            };

            // The trace spans the terminal width (one point per column), not
            // the spectrum's bar count: on a left/right layout the bar count
            // is derived from the height axis, and a ~24-point trace stretched
            // over 100 columns reads as a pixelated blur. With one point per
            // column the renderer's envelope branch downsamples the window
            // with true per-column min/max (DAW-style) instead of aliasing.
            let trace_len = {
                let (w, _) = renderer.dims();
                if w > 0 { w as usize } else { num_bars.max(1) }
            };
            let mut wave_bars = vec![0.0f64; trace_len];
            if mono.len() >= 2 && trace_len > 0 {
                // Scope bandwidth. A raw music mix (hi-hat, 440-880 Hz
                // arpeggio) turns the trace into pixelated mush and gives the
                // trigger endless spurious crossings, so the input is low-
                // passed before it enters the ring. Noise Reduction drives the
                // cutoff: 0 is near-raw, 1 collapses to a clean kick/bass wave.
                let nr = config.smoothing.noise_reduction.clamp(0.0, 1.0);
                let fc = 150.0 + (1.0 - nr).powi(2) * 3000.0f64;
                let a = lowpass_coeff(fc, effective_rate as f64);
                let mono: Vec<f64> = mono
                    .into_iter()
                    .map(|s| {
                        wave_lp1 += a * (s - wave_lp1);
                        wave_lp2 += a * (wave_lp1 - wave_lp2);
                        wave_lp2
                    })
                    .collect();

                // Feed the rolling history and take a triggered window from it.
                wave_ring.extend_from_slice(&mono);
                const WAVE_RING_CAP: usize = 8192; // ~186 ms @ 44.1 kHz
                if wave_ring.len() > WAVE_RING_CAP {
                    wave_ring.drain(..wave_ring.len() - WAVE_RING_CAP);
                }
                const WAVE_WINDOW: usize = 1024; // ~23 ms @ 44.1 kHz
                let ring_peak = wave_ring.iter().fold(0.0f64, |m, s| m.max(s.abs()));
                let start = wave_trigger_start(&wave_ring, WAVE_WINDOW, ring_peak * 0.25)
                    .unwrap_or(wave_ring.len().saturating_sub(WAVE_WINDOW));
                let window: Vec<f64> = wave_ring[start..]
                    .iter()
                    .take(WAVE_WINDOW)
                    .copied()
                    .collect();

                // Auto-gain the trace like the spectrum's autosens: without it
                // a quiet signal only sweeps the middle few percent of the
                // screen and the scope reads as broken.
                let peak = window.iter().fold(0.0f64, |m, s| m.max(s.abs()));
                let (p, g) = auto_gain(peak, wave_peak, wave_gain);
                wave_peak = p;
                wave_gain = g;
                let gain = wave_gain;

                // Box-average the *window* onto the trace length (one evenly
                // sized bin per column) instead of 1-sample picking: a naive
                // nearest/lerp take of a 1024-sample window at ~10 samples per
                // column aliases high-frequency music into a static-ish noise
                // trace. The window is phase-locked by the trigger, so
                // consecutive frames still show the same portion of the wave.
                let downsampled = downsample_box_average(&window, trace_len);
                for (n, sample) in downsampled.iter().enumerate() {
                    let val = (sample * gain * user_sensitivity + 1.0) * 0.5;
                    wave_bars[n] = val.clamp(0.0, 1.0);
                }
            }
            wave_bars
        } else {
            let mut dsp_out = dsp_plan.execute(&samples);

            // Apply manual user sensitivity directly from Up/Down keys and config
            for v in dsp_out.iter_mut() {
                *v = (*v * user_sensitivity).clamp(0.0, 1.0);
            }

            // Apply user EQ
            for ch in 0..audio_channels {
                for n in 0..bars_per_audio_chan {
                    let idx = ch * bars_per_audio_chan + n;
                    if idx < dsp_out.len() && n < user_eq.len() {
                        dsp_out[idx] *= user_eq[n];
                    }
                }
            }

            // Monstercat & Wave smoothing. The filter's `height` argument only
            // feeds the `waves` variant's normalizer; pass the real terminal
            // height instead of a made-up constant so the quadratic falloff
            // stays proportional to what is actually on screen.
            if config.smoothing.monstercat > 0.0 || config.smoothing.waves {
                let term_height = f64::from(renderer.dims().1).max(1.0);
                if audio_channels == 2 {
                    apply_monstercat_filter(
                        &mut dsp_out[0..bars_per_audio_chan],
                        config.smoothing.waves,
                        config.smoothing.monstercat,
                        term_height,
                    );
                    apply_monstercat_filter(
                        &mut dsp_out[bars_per_audio_chan..2 * bars_per_audio_chan],
                        config.smoothing.waves,
                        config.smoothing.monstercat,
                        term_height,
                    );
                } else {
                    apply_monstercat_filter(
                        &mut dsp_out,
                        config.smoothing.waves,
                        config.smoothing.monstercat,
                        term_height,
                    );
                }
            }

            // Stereo / Mono channel layout mapping
            let mut out = vec![0.0f64; num_bars];
            if audio_channels == 2 {
                let left = &dsp_out[0..bars_per_audio_chan];
                let right = &dsp_out[bars_per_audio_chan..2 * bars_per_audio_chan];

                if output_channels == 2 {
                    let half = bars_per_audio_chan;
                    for n in 0..half {
                        if config.output.split_stereo {
                            // Split: [Low...High | Low...High]
                            out[n] = left[n];
                            out[n + half] = right[n];
                        } else if config.output.reverse {
                            // Reverse Mirrored: [Low...High | High...Low]
                            out[n] = left[n];
                            out[n + half] = right[half - 1 - n];
                        } else {
                            // Mirrored (Centered): [High...Low | Low...High]
                            out[n] = left[half - 1 - n];
                            out[n + half] = right[n];
                        }
                    }
                } else {
                    // Mono output from stereo audio input
                    for n in 0..num_bars {
                        let l_val = if n < left.len() { left[n] } else { 0.0 };
                        let r_val = if n < right.len() { right[n] } else { 0.0 };
                        let val = match config.output.mono_option {
                            MonoOption::Left => l_val,
                            MonoOption::Right => r_val,
                            MonoOption::Average => (l_val + r_val) / 2.0,
                        };
                        let target_idx = if config.output.reverse { num_bars - 1 - n } else { n };
                        out[target_idx] = val;
                    }
                }
            } else {
                // Mono audio input
                for n in 0..num_bars {
                    let val = if n < dsp_out.len() { dsp_out[n] } else { 0.0 };
                    let target_idx = if config.output.reverse { num_bars - 1 - n } else { n };
                    out[target_idx] = val;
                }
            }

            out
        };

        // Render bars
        let freqs = &dsp_plan.freq_plan.cut_off_frequency;
        renderer.render(&final_bars, &config, Some(freqs))?;

        // Rate limiting
        let elapsed = loop_start.elapsed();
        if elapsed < frame_interval {
            thread::sleep(frame_interval - elapsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{auto_gain, downsample_box_average, lowpass_coeff, wave_trigger_start};

    /// The trigger must anchor the window on a rising zero crossing so
    /// consecutive frames show the same phase of a repeating waveform.
    #[test]
    fn trigger_finds_latest_rising_zero_crossing() {
        let n = 4096;
        let sig: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 8.0 * i as f64 / n as f64).sin())
            .collect();
        let start = wave_trigger_start(&sig, 1024, 0.0).expect("crossing exists");
        assert!(start >= 1, "start must have a predecessor sample");
        assert!(start + 1024 <= n, "a full window must remain");
        assert!(
            sig[start - 1] <= 0.0 && sig[start] > 0.0,
            "start is not on a rising zero crossing"
        );
    }

    /// Silence has no strict crossing; the scope must still show the newest
    /// window instead of nothing.
    #[test]
    fn trigger_falls_back_when_no_crossing() {
        let sig = vec![0.0f64; 4096];
        let start = wave_trigger_start(&sig, 1024, 0.0).expect("fallback");
        assert_eq!(start + 1024, sig.len());
    }

    /// A DC-positive signal has no rising crossing either — same fallback.
    #[test]
    fn trigger_falls_back_for_dc_offset() {
        let sig = vec![0.5f64; 2048];
        let start = wave_trigger_start(&sig, 1024, 0.0).expect("fallback");
        assert_eq!(start + 1024, sig.len());
    }

    #[test]
    fn trigger_none_when_ring_too_short() {
        assert_eq!(wave_trigger_start(&[0.5, -0.5], 1024, 0.0), None);
        assert_eq!(wave_trigger_start(&[], 1, 0.0), None);
    }

    /// The chosen window must be exactly `window` samples long.
    #[test]
    fn trigger_window_fits() {
        let sig: Vec<f64> = (0..3000)
            .map(|i| (i as f64 * 0.01).sin())
            .collect();
        let start = wave_trigger_start(&sig, 1000, 0.0).unwrap();
        assert!(start + 1000 <= sig.len());
    }

    /// Hysteresis must ignore shallow noise wiggles around zero: with a real
    /// deep-negative crossing available, the trigger picks it rather than the
    /// late small ripple.
    #[test]
    fn trigger_hysteresis_skips_shallow_wiggles() {
        let mut sig = vec![0.0f64; 2048];
        // A real crossing at index 100: comes from a deep negative dip.
        sig[99] = -1.0;
        sig[100] = 0.5;
        // A shallow wiggle at index 1900: -0.05 -> +0.05, must be rejected.
        sig[1899] = -0.05;
        sig[1900] = 0.05;
        let start = wave_trigger_start(&sig, 100, 0.25).expect("real crossing");
        assert_eq!(start, 100, "shallow wiggle captured the trigger");
    }

    /// The whole point of the trigger: a steady periodic tone must produce the
    /// *same* window on consecutive frames. A hysteresis condition that needs a
    /// single-sample jump can never fire on a smooth signal and silently
    /// degrades to the scrolling fallback — this catches that.
    #[test]
    fn trigger_freezes_periodic_signal_across_frames() {
        let fs = 44100.0;
        let window = 1024;
        let mut ring: Vec<f64> = Vec::new();
        let mut idx = 0u64;
        let mut prev: Option<Vec<f64>> = None;
        let mut stable = 0;
        for _frame in 0..8 {
            let chunk: Vec<f64> = (0..512)
                .map(|_| {
                    let t = idx as f64 / fs;
                    idx += 1;
                    (2.0 * std::f64::consts::PI * 110.0 * t).sin()
                })
                .collect();
            ring.extend_from_slice(&chunk);
            if ring.len() > 8192 {
                ring.drain(..ring.len() - 8192);
            }
            if ring.len() < window {
                continue;
            }
            let peak = ring.iter().fold(0.0f64, |m, s| m.max(s.abs()));
            let start = wave_trigger_start(&ring, window, peak * 0.25).unwrap();
            let win = ring[start..start + window].to_vec();
            if let Some(p) = &prev {
                let max_diff = win.iter().zip(p).fold(0.0f64, |m, (a, b)| m.max((a - b).abs()));
                // Sub-sample phase drift makes the windows near-identical, not
                // bit-identical; a scrolling fallback would differ by ~2.0.
                if max_diff < 0.1 {
                    stable += 1;
                }
            }
            prev = Some(win);
        }
        assert!(stable >= 5, "triggered window did not freeze ({stable} stable frames)");
    }

    /// A one-pole cascade at 300 Hz must pass a 100 Hz tone but visibly
    /// attenuate a 1 kHz one, which is what cleans up the scope trace.
    #[test]
    fn lowpass_coeff_attenuates_above_cutoff() {
        let fs = 44100.0;
        let a = lowpass_coeff(300.0, fs);
        // Two cascaded one-poles: steady-state gain at frequency f.
        let gain = |f: f64| {
            let w = 2.0 * std::f64::consts::PI * f / fs;
            let d = 1.0 - 2.0 * (1.0 - a) * w.cos() + (1.0 - a) * (1.0 - a);
            let one = a / d.sqrt();
            one * one
        };
        assert!(gain(100.0) > 0.6, "100 Hz should pass, got {}", gain(100.0));
        assert!(gain(1000.0) < 0.1, "1 kHz should be cut, got {}", gain(1000.0));
    }

    /// A one-frame peak spike must not drag the gain down immediately, and the
    /// gain must ease toward its target instead of stepping.
    #[test]
    fn auto_gain_release_is_slow_and_smoothed() {
        let (peak, gain) = auto_gain(1.0, 0.0, 0.0);
        assert_eq!(peak, 1.0);
        // Gain eases in (0.2 of the way to 0.85), not straight to target.
        assert!(gain > 0.0 && gain < 0.85, "gain stepped instead of easing");
        // A quiet window next frame must not collapse the running peak.
        let (peak2, gain2) = auto_gain(0.1, peak, gain);
        assert!(peak2 > 0.99, "peak released too fast: {peak2}");
        assert!(gain2 >= gain, "gain fell while the running peak held");
    }

    /// Box averaging must not flatten a readably slow sine: the output peak
    /// stays within a few percent of the input peak when the window holds
    /// only a fraction of a cycle (the path that used to alias into noise).
    #[test]
    fn box_average_preserves_slow_sine_peaks() {
        let n = 1024;
        // Two full cycles across the whole window.
        let window: Vec<f64> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * 2.0 * i as f64 / n as f64).sin())
            .collect();
        let out = downsample_box_average(&window, 64);
        assert_eq!(out.len(), 64);
        let out_peak = out.iter().fold(0.0f64, |m, v| m.max(v.abs()));
        assert!(
            (out_peak - 1.0).abs() < 0.15,
            "box-averaged peak {out_peak} drifted too far from the input peak"
        );
        // And the wave must still be visible: sign changes across columns.
        let sign_changes = out.windows(2).filter(|w| (w[0] * w[1]) < 0.0).count();
        assert!(sign_changes >= 2, "slow sine must cross zero in the trace");
    }

    /// A low-frequency mono tonic averages cleanly: every column reads the
    /// same value, which is exactly right for a near-DC input.
    #[test]
    fn box_average_dc_is_constant() {
        let window = vec![0.3f64; 2048];
        let out = downsample_box_average(&window, 4);
        assert!(out.iter().all(|&v| (v - 0.3).abs() < 1e-9));
    }

    #[test]
    fn box_average_interpolates_when_undersampled() {
        let window = vec![0.0f64, 1.0];
        let out = downsample_box_average(&window, 4);
        assert_eq!(out.len(), 4);
        // Monotonic ramp: first two near 0, last two near 1.
        assert!(out[0] < 0.5 && out[1] <= 0.5);
        assert!(out[2] >= 0.5 && out[3] > 0.5);
    }

    #[test]
    fn box_average_empty_yields_empty() {
        assert!(downsample_box_average(&[], 10).is_empty());
        assert!(downsample_box_average(&[1.0, 2.0], 0).is_empty());
    }
}

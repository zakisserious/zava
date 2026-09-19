use zava::config::{Config, InputMethod, Orientation, OutputMethod, ScalingMode};

/// Embedded fixture modelled on a real CAVA-style config. Using a fixture
/// instead of a hard-coded absolute path means these assertions always run
/// rather than silently skipping whenever the file is absent.
const FIXTURE: &str = "\
[general]
framerate = 60
autosens = 1
sensitivity = 100.0
scaling = linear
bars = 0
bar_width = 2
bar_spacing = 1
center_align = true
max_height = 100
lower_cutoff_freq = 50
higher_cutoff_freq = 10000

[input]
method = pipewire
source = auto

[output]
method = noncurses
channels = stereo
show_idle_bar_heads = true

[color]
gradient = 1
gradient_count = 8

[smoothing]
monstercat = 0.0
waves = false
noise_reduction = 77
gravity = 100.0

[eq]
1 = 0.8
2 = 0.9
3 = 1.0
4 = 1.1
5 = 1.2
";

fn fixture_config() -> Config {
    let mut config = Config::default();
    config.parse_ini(FIXTURE);
    config
}

#[test]
fn test_parse_general_section() {
    let config = fixture_config();
    assert_eq!(config.general.framerate, 60);
    assert_eq!(config.general.autosens, 1);
    assert_eq!(config.general.sensitivity, 100.0);
    assert_eq!(config.general.scaling, ScalingMode::Linear);
    assert_eq!(config.general.bar_width, 2);
    assert_eq!(config.general.bar_spacing, 1);
    assert_eq!(config.general.lower_cutoff_freq, 50);
    assert_eq!(config.general.higher_cutoff_freq, 10000);
    assert!(config.general.center_align);
}

#[test]
fn test_max_height_is_a_percentage() {
    let config = fixture_config();
    // CAVA expresses max_height as a percentage; internally it is a fraction.
    assert!(
        (config.general.max_height - 1.0).abs() < 1e-9,
        "max_height = 100 should mean a full-height (1.0) bar, got {}",
        config.general.max_height
    );
}

#[test]
fn test_parse_input_and_output_sections() {
    let config = fixture_config();
    assert_eq!(config.input.method, InputMethod::Pipewire);
    assert_eq!(config.input.source, "auto");
    assert_eq!(config.output.method, OutputMethod::Noncurses);
    assert_eq!(config.output.channels, 2);
    assert!(config.color.gradient);
    assert!(config.output.show_idle_bar_heads);
}

#[test]
fn test_parse_smoothing_section() {
    let config = fixture_config();
    // noise_reduction is given as a percentage and stored as a fraction.
    assert!((config.smoothing.noise_reduction - 0.77).abs() < 1e-9);
    assert!((config.smoothing.gravity - 100.0).abs() < 1e-9);
    assert!((config.smoothing.monstercat - 0.0).abs() < 1e-9);
    assert!(!config.smoothing.waves);
}

#[test]
fn test_eq_values_are_linear_multipliers() {
    let config = fixture_config();
    // CAVA semantics: [eq] values are used verbatim as multipliers and are
    // never converted from decibels.
    assert_eq!(config.eq.get(&1), Some(&0.8));
    assert_eq!(config.eq.get(&3), Some(&1.0));
    assert_eq!(config.eq.get(&5), Some(&1.2));
    assert_eq!(config.eq.len(), 5);
}

#[test]
fn test_eq_negative_and_non_finite_values_are_rejected() {
    let mut config = Config::default();
    config.parse_ini("[eq]\n0 = -3.2\n1 = 1.5\n2 = inf\n");
    // A negative multiplier would invert the bar (which renders as nothing),
    // so it is floored at zero; non-finite input is dropped entirely.
    assert_eq!(config.eq.get(&0), Some(&0.0));
    assert_eq!(config.eq.get(&1), Some(&1.5));
    assert_eq!(config.eq.get(&2), None);
}
#[test]
fn test_save_preserves_comments_and_updates_values() {
    let dir = std::env::temp_dir().join(format!("zava_save_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    std::fs::write(
        &path,
        "# my preamble comment\n[general]\n# framerate comment\nframerate = 30\nbars = 20\n\n[color]\nbackground = #123456\n",
    )
    .unwrap();

    let mut config = Config::load_or_default(Some(&path));
    // Simulate a menu tweak: higher framerate and new bar count.
    config.general.framerate = 120;
    config.general.bars = 50;
    config.save().unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("# my preamble comment"), "preamble comment lost");
    assert!(content.contains("# framerate comment"), "inline comment lost");
    assert!(content.contains("framerate = 120"), "framerate not updated");
    assert!(content.contains("bars = 50"), "bars not updated");
    assert!(content.contains("background = #123456"), "rgb color lost");

    // Reload must parse back to the same values (round-trip).
    let reloaded = Config::load_or_default(Some(&path));
    assert_eq!(reloaded.general.framerate, 120);
    assert_eq!(reloaded.general.bars, 50);

    // A second save with no changes must be stable (idempotent).
    let before = std::fs::read_to_string(&path).unwrap();
    let mut reloaded2 = reloaded;
    reloaded2.save().unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before, after, "save is not idempotent");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_save_appends_missing_sections() {
    let dir = std::env::temp_dir().join(format!("zava_save_append_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    // Minimal file: only one key. Everything else must be appended.
    std::fs::write(&path, "[general]\nframerate = 30\n").unwrap();

    let mut config = Config::load_or_default(Some(&path));
    config.save().unwrap();

    let content = std::fs::read_to_string(&path).unwrap();
    for section in ["[input]", "[output]", "[color]", "[smoothing]"] {
        assert!(content.contains(section), "missing section {}", section);
    }
    assert!(content.contains("orientation = bottom"), "orientation missing");
    assert!(content.contains("gravity = "), "gravity missing");

    // Reload parses cleanly (no INI errors).
    let reloaded = Config::load_or_default(Some(&path));
    assert_eq!(reloaded.general.framerate, 30);
    assert_eq!(reloaded.output.orientation, Orientation::Bottom);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn test_legacy_cava_configs_cannot_request_vertical() {
    // `vertical` was a cut orientation (mirrored centre-column bars using
    // half-width glyphs). Existing configs naming it fall back to the classic
    // bottom-up layout instead of an unrenderable variant.
    let mut config = Config::default();
    config.parse_ini("[output]\norientation = vertical\n");
    assert_eq!(config.output.orientation, Orientation::Bottom);
}

#[test]
fn test_zava_config_serializes_orientation_and_wrap_bars() {
    // The orientation and the new bar-continuation flag round-trip through
    // save so they survive menu edits, PTY restarts and release packaging.
    let dir = std::env::temp_dir().join(format!("zava_orient_test_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("config");
    std::fs::write(&path, "[output]\nchannels = stereo\n").unwrap();

    let mut config = Config::load_or_default(Some(&path));
    config.output.orientation = Orientation::Horizontal;
    config.general.wrap_bars = true;
    config.save().unwrap();

    let reloaded = Config::load_or_default(Some(&path));
    assert_eq!(reloaded.output.orientation, Orientation::Horizontal);
    assert!(reloaded.general.wrap_bars);

    let content = std::fs::read_to_string(&path).unwrap();
    assert!(content.contains("orientation = horizontal"));
    assert!(content.contains("wrap_bars"));

    std::fs::remove_dir_all(&dir).unwrap();
}

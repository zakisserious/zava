use std::f64::consts::PI;
use zava::config::ScalingMode;
use zava::dsp::filter::apply_monstercat_filter;
use zava::dsp::CavaPlan;

#[test]
fn test_cavacore_simulation() {
    let bars_per_channel = 10;
    let channels = 2;
    let buffer_size = 512 * channels;
    let rate = 44100;
    let noise_reduction = 0.77;
    let low_cut_off = 50;
    let high_cut_off = 10000;

    let mut plan = CavaPlan::new(
        bars_per_channel,
        rate,
        channels,
        1,
        noise_reduction,
        low_cut_off,
        high_cut_off,
        ScalingMode::Linear,
        100.0,
    )
    .expect("Plan initialization failed");

    let mut cava_in = vec![0.0f64; buffer_size];
    let mut cava_out = vec![0.0f64; bars_per_channel * channels];

    for k in 0..300 {
        for n in 0..(buffer_size / 2) {
            let t = (n + (k * buffer_size / 2)) as f64 / rate as f64;
            cava_in[n * 2] = (2.0 * PI * 200.0 * t).sin() * 20000.0;
            cava_in[n * 2 + 1] = (2.0 * PI * 2000.0 * t).sin() * 20000.0;
        }
        cava_out = plan.execute(&cava_in);
    }

    // Peak for 200 Hz is at bar index 2 (approx 0.990)
    let (max_200hz_idx, &max_200hz_val) = cava_out[0..bars_per_channel]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();
    assert_eq!(max_200hz_idx, 2, "Peak for 200 Hz should be at bar 2");
    assert!(max_200hz_val > 0.95, "Expected ~0.994, got {max_200hz_val}");

    // Peak for 2000 Hz is at bar index 6 (approx 0.680)
    let (max_2000hz_idx, &max_2000hz_val) = cava_out[bars_per_channel..2 * bars_per_channel]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap();
    assert_eq!(max_2000hz_idx, 6, "Peak for 2000 Hz should be at bar 6");
    assert!(max_2000hz_val > 0.65 && max_2000hz_val < 0.72, "Expected ~0.683, got {max_2000hz_val}");
}

#[test]
fn test_monstercat_smoothing() {
    let mut bars = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
    apply_monstercat_filter(&mut bars, false, 1.0, 100.0);
    assert_eq!(bars[3], 1.0);
    assert!(bars[2] > 0.0 && bars[2] < 1.0);
    assert!(bars[4] > 0.0 && bars[4] < 1.0);
    assert!(bars[1] > 0.0 && bars[1] < bars[2]);
    assert!(bars[5] > 0.0 && bars[5] < bars[4]);
}

#[test]
fn test_autosens_reaches_full_height() {
    // Mirrors the C reference harness: 26 bars x 2 channels, deterministic
    // white noise, autosens enabled. The autosens controller should settle
    // with the loudest bar near full height (>= 0.9).
    let bars_per_channel = 13;
    let channels = 2;
    let rate = 44100u32;
    let mut plan = CavaPlan::new(
        bars_per_channel, rate, channels, 1, 0.77, 50, 10000,
        ScalingMode::Linear, 100.0,
    ).expect("Plan initialization failed");

    let nframes = rate * 10;
    let mut st: u32 = 12345;
    let chunk_frames = 1024;
    let mut chunk = vec![0.0f64; chunk_frames * channels];
    let mut filled = 0usize;
    let mut maxes: Vec<f64> = Vec::new();
    for _ in 0..nframes {
        st = st.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
        let v = ((st as f64 / 2147483647.0) * 2.0 - 1.0) * 28000.0;
        chunk[filled * 2] = v;
        chunk[filled * 2 + 1] = v;
        filled += 1;
        if filled == chunk_frames {
            let out = plan.execute(&chunk);
            let m = out.iter().cloned().fold(0.0f64, f64::max);
            maxes.push(m);
            filled = 0;
        }
    }
    assert!(!maxes.is_empty(), "expected at least one execute");
    // Use the mean of the last 60 executes: the autosens controller breathes
    // around its set point, so a single frame can dip below 0.9.
    let tail = &maxes[maxes.len().saturating_sub(60)..];
    let mean: f64 = tail.iter().sum::<f64>() / tail.len() as f64;
    assert!(
        mean > 0.9,
        "autosens steady-state mean max should exceed 0.9, got {mean}"
    );
}


#[test]
fn test_interpolate_user_eq_matches_cava_semantics() {
    use std::collections::BTreeMap;
    use zava::dsp::filter::interpolate_user_eq;

    // Values are raw linear multipliers sampled as floor(n * points / bars),
    // exactly like CAVA's output/common.c.
    let mut eq = BTreeMap::new();
    eq.insert(1, 0.8);
    eq.insert(2, 0.9);
    eq.insert(3, 1.0);
    eq.insert(4, 1.1);
    eq.insert(5, 1.2);
    // ratio = 5 / 10 = 0.5 -> point indices 0,0,1,1,2,2,3,3,4,4
    assert_eq!(
        interpolate_user_eq(&eq, 10),
        vec![0.8, 0.8, 0.9, 0.9, 1.0, 1.0, 1.1, 1.1, 1.2, 1.2]
    );

    // Unset EQ leaves every band at unity.
    let empty: BTreeMap<usize, f64> = BTreeMap::new();
    assert_eq!(interpolate_user_eq(&empty, 4), vec![1.0; 4]);

    // A single control point applies across the whole spectrum.
    let mut one = BTreeMap::new();
    one.insert(0, 0.5);
    assert_eq!(interpolate_user_eq(&one, 3), vec![0.5; 3]);

    // A negative gain must never invert a band (which would render as nothing).
    let mut negative = BTreeMap::new();
    negative.insert(0, -3.2);
    negative.insert(1, 0.0);
    negative.insert(2, 1.5);
    let out = interpolate_user_eq(&negative, 3);
    assert!(
        out.iter().all(|v| *v >= 0.0),
        "negative gains must be floored at zero, got {out:?}"
    );
}

#[test]
fn test_loud_level_jump_recovers() {
    // A jump from a quiet passage to full volume must not leave the display
    // pinned at the ceiling. The autosens decay has to shed the excess gain
    // within a fraction of a second; a rate tuned slow enough to look stable
    // at equilibrium instead leaves every band clamped at the top for tens of
    // seconds, which reads as "no reactivity when the volume is turned up".
    //
    // Reference: CAVA 1.0.0 on this exact input peaks around 22/26 bars pinned
    // shortly after the jump and settles near 14/26, so the thresholds below
    // are deliberately loose enough to accept CAVA-equivalent behaviour.
    let bars_per_channel = 13;
    let channels = 2;
    let rate = 44100u32;
    let mut plan = CavaPlan::new(
        bars_per_channel,
        rate,
        channels,
        1,
        0.77,
        50,
        10000,
        ScalingMode::Linear,
        100.0,
    )
    .expect("plan initialization failed");

    let chunk_frames = 1024usize;
    let mut state: u32 = 999;
    let mut next_noise = move || {
        state = state.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
        (state as f64 / 2147483647.0) * 2.0 - 1.0
    };

    // Settle at a quiet level first.
    let mut chunk = vec![0.0f64; chunk_frames * channels];
    let mut settled = Vec::new();
    for _ in 0..600 {
        for i in 0..chunk_frames {
            let v = next_noise() * 800.0;
            chunk[i * 2] = v;
            chunk[i * 2 + 1] = v;
        }
        settled = plan.execute(&chunk);
    }
    let settled_max = settled.iter().cloned().fold(0.0f64, f64::max);
    assert!(
        settled_max > 0.8,
        "autosens should settle near full height, got {settled_max}"
    );

    // Slam the level: 40x louder, i.e. "volume to maximum".
    let mut loud = vec![0.0f64; chunk_frames * channels];
    for i in 0..chunk_frames {
        let v = next_noise() * 32000.0;
        loud[i * 2] = v;
        loud[i * 2 + 1] = v;
    }
    let mut last = Vec::new();
    for _ in 0..300 {
        last = plan.execute(&loud);
    }

    let total = last.len();
    let pinned = last.iter().filter(|v| **v >= 0.999).count();
    let hi = last.iter().cloned().fold(0.0f64, f64::max);
    let lo = last.iter().cloned().fold(f64::INFINITY, f64::min);

    assert!(
        hi > 0.8,
        "loud input should still reach near full height, got {hi}"
    );
    assert!(
        hi - lo > 0.2,
        "loud input left no visible dynamic range: lo={lo} hi={hi}"
    );
    assert!(
        pinned < total / 2,
        "5s after a 40x level jump {pinned}/{total} bars are still pinned at the ceiling; \
autosens is not shedding gain fast enough"
    );
}

#[test]
fn dump_freq_plan() {
    let plan = zava::dsp::filter::calculate_frequency_plan(26, 44100, 8192, 4096, 50, 10000);
    for n in 0..26 {
        println!(
            "bar {:2}: low={:4} up={:4} eq={:.10e} cutoff={:.1}",
            n,
            plan.fft_buffer_lower_cut_off[n],
            plan.fft_buffer_upper_cut_off[n],
            plan.eq[n],
            plan.cut_off_frequency[n]
        );
    }
}

#[test]
fn dump_frames_autosens_off() {
    let as_on = std::env::var("ZAVA_AS_ON").is_ok();
    let bars_per_channel = 13;
    let channels = 2;
    let rate = 44100u32;
    let mut plan = CavaPlan::new(
        bars_per_channel, rate, channels, if as_on {1} else {0}, 0.77, 50, 10000,
        ScalingMode::Linear, 100.0,
    ).expect("plan");

    let nframes = rate * 10;
    let mut st: u32 = 12345;
    let chunk_frames = 1024usize;
    let mut chunk = vec![0.0f64; chunk_frames * channels];
    let mut filled = 0usize;
    use std::io::Write;
    let mut f = std::io::BufWriter::new(std::fs::File::create(if as_on {"/tmp/zava_frames_a1.bin"} else {"/tmp/zava_frames.bin"}).unwrap());
    for _ in 0..nframes {
        st = st.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
        let v = ((st as f64 / 2147483647.0) * 2.0 - 1.0) * 28000.0;
        chunk[filled * 2] = v;
        chunk[filled * 2 + 1] = v;
        filled += 1;
        if filled == chunk_frames {
            let out = plan.execute(&chunk);
            let mut b = Vec::with_capacity(out.len() * 8);
            for x in &out { b.extend_from_slice(&x.to_ne_bytes()); }
            f.write_all(&b).unwrap();
            filled = 0;
        }
    }
    f.flush().unwrap();
}


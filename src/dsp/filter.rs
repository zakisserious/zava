use std::collections::BTreeMap;
use std::f64::consts::PI;

pub struct FrequencyPlan {
    pub cut_off_frequency: Vec<f64>,
    pub fft_buffer_lower_cut_off: Vec<usize>,
    pub fft_buffer_upper_cut_off: Vec<usize>,
    pub bass_cut_off_bar: usize,
    pub eq: Vec<f64>,
}

pub fn calculate_frequency_plan(
    number_of_bars: usize,
    rate: u32,
    fft_bass_size: usize,
    fft_buffer_size: usize,
    low_cut_off: u32,
    high_cut_off: u32,
) -> FrequencyPlan {
    let lower_cut_off = low_cut_off as f64;
    let upper_cut_off = high_cut_off as f64;
    let bass_cut_off = 100.0;
    let rate_f = rate as f64;

    let frequency_constant = (lower_cut_off / upper_cut_off).log10()
        / (1.0 / (number_of_bars as f64 + 1.0) - 1.0);

    let min_bandwidth = rate_f / fft_bass_size as f64;

    let mut cut_off_frequency = vec![0.0; number_of_bars + 1];
    let mut relative_cut_off = vec![0.0; number_of_bars + 1];
    let mut fft_buffer_lower_cut_off = vec![0; number_of_bars + 1];
    let mut fft_buffer_upper_cut_off = vec![0; number_of_bars + 1];
    let mut bass_cut_off_bar = 0;
    let mut first_bar = true;

    for n in 0..=number_of_bars {
        let mut bar_distribution_coeff = -frequency_constant;
        bar_distribution_coeff += ((n as f64 + 1.0) / (number_of_bars as f64 + 1.0)) * frequency_constant;
        cut_off_frequency[n] = upper_cut_off * 10.0f64.powf(bar_distribution_coeff);

        if n > 0 && cut_off_frequency[n - 1] >= cut_off_frequency[n] {
            cut_off_frequency[n] = cut_off_frequency[n - 1] + min_bandwidth;
        }

        relative_cut_off[n] = cut_off_frequency[n] / (rate_f / 2.0);

        if cut_off_frequency[n] < bass_cut_off {
            fft_buffer_lower_cut_off[n] = (relative_cut_off[n] * (fft_bass_size as f64 / 2.0)).floor() as usize;
            bass_cut_off_bar += 1;
            if bass_cut_off_bar > 1 {
                first_bar = false;
            }
            if fft_buffer_lower_cut_off[n] > fft_bass_size / 2 {
                fft_buffer_lower_cut_off[n] = fft_bass_size / 2;
            }
        } else {
            fft_buffer_lower_cut_off[n] = (relative_cut_off[n] * (fft_buffer_size as f64 / 2.0)).ceil() as usize;
            if n == bass_cut_off_bar {
                first_bar = true;
                if n > 0 {
                    let val = (relative_cut_off[n] * (fft_bass_size as f64 / 2.0) - 1.0).max(0.0) as usize;
                    fft_buffer_upper_cut_off[n - 1] = val;
                }
            } else {
                first_bar = false;
            }
            if fft_buffer_lower_cut_off[n] > fft_buffer_size / 2 {
                fft_buffer_lower_cut_off[n] = fft_buffer_size / 2;
            }
        }

        if n > 0 {
            if !first_bar {
                fft_buffer_upper_cut_off[n - 1] = fft_buffer_lower_cut_off[n].saturating_sub(1);
                if fft_buffer_lower_cut_off[n] <= fft_buffer_lower_cut_off[n - 1] {
                    let room_for_more = if n < bass_cut_off_bar {
                        fft_buffer_lower_cut_off[n - 1] + 1 < fft_bass_size / 2 + 1
                    } else {
                        fft_buffer_lower_cut_off[n - 1] + 1 < fft_buffer_size / 2 + 1
                    };
                    if room_for_more {
                        fft_buffer_lower_cut_off[n] = fft_buffer_lower_cut_off[n - 1] + 1;
                        fft_buffer_upper_cut_off[n - 1] = fft_buffer_lower_cut_off[n].saturating_sub(1);
                    }
                }
            } else if fft_buffer_upper_cut_off[n - 1] < fft_buffer_lower_cut_off[n - 1] {
                fft_buffer_upper_cut_off[n - 1] = fft_buffer_lower_cut_off[n - 1] + 1;
            }
        }

        if n < bass_cut_off_bar {
            relative_cut_off[n] = fft_buffer_lower_cut_off[n] as f64 / (fft_bass_size as f64 / 2.0);
        } else {
            relative_cut_off[n] = fft_buffer_lower_cut_off[n] as f64 / (fft_buffer_size as f64 / 2.0);
        }
        cut_off_frequency[n] = relative_cut_off[n] * (rate_f / 2.0);
    }

    let mut eq = vec![0.0; number_of_bars];
    for n in 0..number_of_bars {
        let mut eq_val = 1.0 / 2.0f64.powi(28);
        eq_val *= cut_off_frequency[n + 1].powf(0.85);

        if n < bass_cut_off_bar {
            eq_val /= (fft_bass_size as f64).log2();
        } else {
            eq_val /= (fft_buffer_size as f64).log2();
        }

        let bins_count = (fft_buffer_upper_cut_off[n] as isize - fft_buffer_lower_cut_off[n] as isize + 1).max(1) as f64;
        eq_val /= bins_count;
        eq[n] = eq_val;
    }

    FrequencyPlan {
        cut_off_frequency,
        fft_buffer_lower_cut_off,
        fft_buffer_upper_cut_off,
        bass_cut_off_bar,
        eq,
    }
}

pub fn create_hann_window(size: usize) -> Vec<f64> {
    let mut window = vec![0.0; size];
    let size_f = size as f64;
    for (i, w) in window.iter_mut().enumerate() {
        *w = 0.5 * (1.0 - (2.0 * PI * i as f64 / (size_f - 1.0)).cos());
    }
    window
}

pub fn apply_monstercat_filter(bars: &mut [f64], waves: bool, monstercat: f64, height: f64) {
    let number_of_bars = bars.len();
    if number_of_bars == 0 {
        return;
    }

    let height_normalizer = if height > 1000.0 {
        height / 912.76
    } else {
        1.0
    };

    if waves {
        for z in 0..number_of_bars {
            bars[z] /= 1.25;
            for m_y in (0..z).rev() {
                let de = (z - m_y) as f64;
                let val = bars[z] - height_normalizer * de * de;
                if val > bars[m_y] {
                    bars[m_y] = val;
                }
            }
            for m_y in (z + 1)..number_of_bars {
                let de = (m_y - z) as f64;
                let val = bars[z] - height_normalizer * de * de;
                if val > bars[m_y] {
                    bars[m_y] = val;
                }
            }
        }
    } else if monstercat > 0.0 {
        let base = monstercat * 1.5;
        for z in 0..number_of_bars {
            for m_y in (0..z).rev() {
                let de = (z - m_y) as f64;
                let val = bars[z] / base.powf(de);
                if val > bars[m_y] {
                    bars[m_y] = val;
                }
            }
            for m_y in (z + 1)..number_of_bars {
                let de = (m_y - z) as f64;
                let val = bars[z] / base.powf(de);
                if val > bars[m_y] {
                    bars[m_y] = val;
                }
            }
        }
    }
}

/// Maps the user `[eq]` control points onto the rendered bars, matching CAVA's
/// `output/common.c` exactly.
///
/// The values are **raw linear multipliers**, not decibels: `1.0` is unity,
/// `0.8` attenuates by 20%, `1.2` boosts by 20%. This is the same convention as
/// CAVA, which multiplies `cava_out[n]` by the value straight from the config.
///
/// Each control point covers an equal-width slice of the spectrum, sampled with
/// `floor(n * points / bars)` exactly like CAVA. The numeric keys in the config
/// file are only used to order the points (CAVA ignores their values too and
/// reads them in file order).
pub fn interpolate_user_eq(user_eq: &BTreeMap<usize, f64>, number_of_bars: usize) -> Vec<f64> {
    if user_eq.is_empty() || number_of_bars == 0 {
        return vec![1.0; number_of_bars];
    }
    // Guard against values that would invert or blank a band outright. A
    // negative gain mirrors the bar below the baseline, which renders as
    // nothing, so floor it at zero rather than silently producing dead bars.
    let points: Vec<f64> = user_eq
        .values()
        .copied()
        .map(|v| if v.is_finite() { v.max(0.0) } else { 1.0 })
        .collect();
    let num_points = points.len();
    if num_points == 1 || number_of_bars == 1 {
        return vec![points[0]; number_of_bars];
    }
    let ratio = num_points as f64 / number_of_bars as f64;
    (0..number_of_bars)
        .map(|n| {
            let idx = ((n as f64 * ratio).floor() as usize).min(num_points - 1);
            points[idx]
        })
        .collect()
}

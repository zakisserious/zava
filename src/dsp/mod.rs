pub mod filter;

use std::sync::Arc;
use rustfft::num_complex::Complex64;
use rustfft::{Fft, FftPlanner};
use crate::config::ScalingMode;
use self::filter::{
    calculate_frequency_plan, create_hann_window, FrequencyPlan,
};

pub struct CavaPlan {
    pub number_of_bars: usize,
    pub audio_channels: usize,
    pub rate: u32,
    pub autosens: u32,
    pub noise_reduction: f64,
    pub gravity: f64,
    pub scaling_mode: ScalingMode,

    pub fft_bass_size: usize,
    pub fft_buffer_size: usize,
    pub input_buffer_size: usize,

    input_buffer: Vec<f64>,
    bass_window: Vec<f64>,
    main_window: Vec<f64>,

    fft_bass: Arc<dyn Fft<f64>>,
    fft_main: Arc<dyn Fft<f64>>,

    // Scratch buffers for FFT
    bass_complex_l: Vec<Complex64>,
    bass_complex_r: Vec<Complex64>,
    main_complex_l: Vec<Complex64>,
    main_complex_r: Vec<Complex64>,

    pub freq_plan: FrequencyPlan,

    // Smoothing & physics states
    cava_fall: Vec<f64>,
    cava_mem: Vec<f64>,
    cava_peak: Vec<f64>,
    prev_cava_out: Vec<f64>,

    pub sens: f64,
    sens_init: bool,
    pub framerate: f64,
    frame_skip: u32,
}

impl CavaPlan {
    pub fn new(
        number_of_bars: usize,
        rate: u32,
        channels: usize,
        autosens: u32,
        noise_reduction: f64,
        low_cut_off: u32,
        high_cut_off: u32,
        scaling_mode: ScalingMode,
        gravity: f64,
    ) -> Result<Self, String> {
        if !(1..=2).contains(&channels) {
            return Err(format!("Illegal channels: {channels}, supported: 1 or 2"));
        }
        if rate == 0 || rate > 384000 {
            return Err(format!("Illegal sample rate: {rate}"));
        }
        if number_of_bars == 0 {
            return Err("Number of bars must be positive".to_string());
        }

        let mut fft_buffer_size = 512;
        if rate > 8125 && rate <= 16250 {
            fft_buffer_size *= 2;
        } else if rate > 16250 && rate <= 32500 {
            fft_buffer_size *= 4;
        } else if rate > 32500 && rate <= 75000 {
            fft_buffer_size *= 8;
        } else if rate > 75000 && rate <= 150000 {
            fft_buffer_size *= 16;
        } else if rate > 150000 && rate <= 300000 {
            fft_buffer_size *= 32;
        } else if rate > 300000 {
            fft_buffer_size *= 64;
        }

        let fft_bass_size = fft_buffer_size * 2;
        let input_buffer_size = fft_bass_size * channels;

        let mut planner = FftPlanner::new();
        let fft_bass = planner.plan_fft_forward(fft_bass_size);
        let fft_main = planner.plan_fft_forward(fft_buffer_size);

        let freq_plan = calculate_frequency_plan(
            number_of_bars,
            rate,
            fft_bass_size,
            fft_buffer_size,
            low_cut_off,
            high_cut_off,
        );

        let bass_window = create_hann_window(fft_bass_size);
        let main_window = create_hann_window(fft_buffer_size);

        let total_bars = number_of_bars * channels;

        Ok(Self {
            number_of_bars,
            audio_channels: channels,
            rate,
            autosens,
            noise_reduction,
            gravity,
            scaling_mode,
            fft_bass_size,
            fft_buffer_size,
            input_buffer_size,
            input_buffer: vec![0.0; input_buffer_size],
            bass_window,
            main_window,
            fft_bass,
            fft_main,
            bass_complex_l: vec![Complex64::default(); fft_bass_size],
            bass_complex_r: vec![Complex64::default(); fft_bass_size],
            main_complex_l: vec![Complex64::default(); fft_buffer_size],
            main_complex_r: vec![Complex64::default(); fft_buffer_size],
            freq_plan,
            cava_fall: vec![0.0; total_bars],
            cava_mem: vec![0.0; total_bars],
            cava_peak: vec![0.0; total_bars],
            prev_cava_out: vec![0.0; total_bars],
            sens: 1.0,
            sens_init: true,
            framerate: 75.0,
            frame_skip: 1,
        })
    }

    pub fn execute(&mut self, cava_in: &[f64]) -> Vec<f64> {
        let mut new_samples = cava_in.len();
        if new_samples > self.input_buffer_size {
            new_samples = self.input_buffer_size;
        }

        let mut silence = true;

        if new_samples > 0 {
            // Approximate actual framerate
            self.framerate -= self.framerate / 64.0;
            self.framerate += (self.rate as f64 * self.frame_skip as f64)
                / (new_samples as f64 / self.audio_channels as f64)
                / 64.0;
            self.frame_skip = 1;

            // Shift input buffer
            if new_samples < self.input_buffer_size {
                self.input_buffer.copy_within(0..(self.input_buffer_size - new_samples), new_samples);
            }

            // Fill input buffer with new samples in reverse order
            for n in 0..new_samples {
                let sample = cava_in[n];
                if sample != 0.0 {
                    silence = false;
                }
                let val = if self.scaling_mode == ScalingMode::Decibel {
                    sample / 32768.0
                } else {
                    sample
                };
                self.input_buffer[new_samples - n - 1] = val;
            }
        } else {
            self.frame_skip += 1;
        }

        // Window into bass and main FFT buffers
        for n in 0..self.fft_bass_size {
            let (raw_l, raw_r) = if self.audio_channels == 2 {
                (self.input_buffer[n * 2 + 1], self.input_buffer[n * 2])
            } else {
                (self.input_buffer[n], 0.0)
            };
            self.bass_complex_l[n] = Complex64::new(raw_l * self.bass_window[n], 0.0);
            if self.audio_channels == 2 {
                self.bass_complex_r[n] = Complex64::new(raw_r * self.bass_window[n], 0.0);
            }
        }

        for n in 0..self.fft_buffer_size {
            let (raw_l, raw_r) = if self.audio_channels == 2 {
                (self.input_buffer[n * 2 + 1], self.input_buffer[n * 2])
            } else {
                (self.input_buffer[n], 0.0)
            };
            self.main_complex_l[n] = Complex64::new(raw_l * self.main_window[n], 0.0);
            if self.audio_channels == 2 {
                self.main_complex_r[n] = Complex64::new(raw_r * self.main_window[n], 0.0);
            }
        }

        // Run FFT
        self.fft_bass.process(&mut self.bass_complex_l);
        self.fft_main.process(&mut self.main_complex_l);
        if self.audio_channels == 2 {
            self.fft_bass.process(&mut self.bass_complex_r);
            self.fft_main.process(&mut self.main_complex_r);
        }

        let mut cava_out = vec![0.0; self.number_of_bars * self.audio_channels];

        // Separate frequency bands
        for n in 0..self.number_of_bars {
            let mut temp_l: f64 = 0.0;
            let mut temp_r: f64 = 0.0;

            let lower = self.freq_plan.fft_buffer_lower_cut_off[n];
            let upper = self.freq_plan.fft_buffer_upper_cut_off[n];

            if n < self.freq_plan.bass_cut_off_bar {
                for i in lower..=upper {
                    if i < self.fft_bass_size / 2 + 1 {
                        temp_l += self.bass_complex_l[i].norm();
                        if self.audio_channels == 2 {
                            temp_r += self.bass_complex_r[i].norm();
                        }
                    }
                }
            } else {
                for i in lower..=upper {
                    if i < self.fft_buffer_size / 2 + 1 {
                        temp_l += self.main_complex_l[i].norm();
                        if self.audio_channels == 2 {
                            temp_r += self.main_complex_r[i].norm();
                        }
                    }
                }
            }

            if self.scaling_mode == ScalingMode::Decibel {
                const MAX_DB: f64 = 70.0;
                temp_l = if temp_l > 0.0 { 20.0 * temp_l.log10() / MAX_DB } else { 0.0 };
                if !temp_l.is_finite() {
                    temp_l = 0.0;
                }
            } else {
                temp_l *= self.freq_plan.eq[n];
            }
            cava_out[n] = temp_l;

            if self.audio_channels == 2 {
                if self.scaling_mode == ScalingMode::Decibel {
                    const MAX_DB: f64 = 70.0;
                    temp_r = if temp_r > 0.0 { 20.0 * temp_r.log10() / MAX_DB } else { 0.0 };
                    if !temp_r.is_finite() {
                        temp_r = 0.0;
                    }
                } else {
                    temp_r *= self.freq_plan.eq[n];
                }
                cava_out[n + self.number_of_bars] = temp_r;
            }
        }

        // Apply autosens multiplier
        if self.autosens != 0 {
            for v in cava_out.iter_mut() {
                *v *= self.sens;
            }
        }

        // Smoothing: Falloff & Integral
        let mut overshoot = false;
        let framerate_mod = 66.0 / self.framerate.max(1.0);
        let nr = self.noise_reduction.max(0.01);
        let gravity_factor = (self.gravity / 100.0).clamp(0.01, 10.0);
        let gravity_mod = framerate_mod.powf(2.5) * 2.0 / nr * gravity_factor;
        let integral_mod = framerate_mod.powf(0.1);

        for n in 0..(self.number_of_bars * self.audio_channels) {
            // Gravity falloff
            if cava_out[n] < self.prev_cava_out[n] && self.noise_reduction > 0.1 {
                cava_out[n] = self.cava_peak[n] * (1.0 - (self.cava_fall[n] * self.cava_fall[n] * gravity_mod));
                if cava_out[n] < 0.0 {
                    cava_out[n] = 0.0;
                }
                self.cava_fall[n] += 0.028;
            } else {
                self.cava_peak[n] = cava_out[n];
                self.cava_fall[n] = 0.0;
            }
            self.prev_cava_out[n] = cava_out[n];

            // Integral smoothing
            cava_out[n] = self.cava_mem[n] * self.noise_reduction / integral_mod + cava_out[n];
            self.cava_mem[n] = cava_out[n];

            if self.autosens != 0 && cava_out[n] > 1.0 {
                overshoot = true;
                cava_out[n] = 1.0;
            }
        }

        // Dynamic sensitivity adjustment
        if self.autosens != 0 {
            if overshoot {
                self.sens *= 1.0 - (0.02 * framerate_mod);
                self.sens_init = false;
            } else if !silence {
                self.sens *= 1.0 + (0.001 * framerate_mod * self.autosens as f64);
                if self.sens_init {
                    self.sens *= 1.0 + (0.1 * framerate_mod);
                }
            }
        }

        cava_out
    }
}

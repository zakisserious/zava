use std::f64::consts::PI;

pub struct TestSignalBackend {
    sample_rate: u32,
    channels: u32,
        sample_index: u64,
}

impl TestSignalBackend {
    pub fn new(sample_rate: u32, channels: u32) -> Self {
        Self {
            sample_rate,
            channels,
                        sample_index: 0,
        }
    }

    pub fn read_samples(&mut self) -> Option<Vec<f64>> {
        let samples_per_frame = 512 * self.channels as usize;
        let mut buffer = vec![0.0f64; samples_per_frame];
        let sr = self.sample_rate as f64;

        for i in 0..512 {
            let t = (self.sample_index + i as u64) as f64 / sr;
            // 128 BPM beat
            let beat_period = 60.0 / 128.0;
            let beat_phase = (t % beat_period) / beat_period;
            let kick_env = (-beat_phase * 12.0).exp();
            let kick = (2.0 * PI * (50.0 + 80.0 * kick_env) * t).sin() * kick_env * 20000.0;

            // Bass pulse
            let bass = (2.0 * PI * 110.0 * t).sin() * 8000.0 * (1.0 + (2.0 * PI * 2.0 * t).sin() * 0.5);

            // Mid arpeggio
            let note = match ((t * 4.0) as usize) % 4 {
                0 => 440.0,
                1 => 554.37,
                2 => 659.25,
                _ => 880.0,
            };
            let mid = (2.0 * PI * note * t).sin() * 6000.0;

            // Hi-hat / high sparkle
            let hat_phase = (t % (beat_period / 2.0)) / (beat_period / 2.0);
            let hat_env = (-hat_phase * 25.0).exp();
            let hat = ((t * 23456.7).sin() * (t * 87654.3).sin()) * hat_env * 7000.0;

            let sample_l = kick + bass + mid + hat;
            let sample_r = kick * 0.9 + bass * 1.1 + mid * 0.8 + hat * 1.2;

            if self.channels == 2 {
                buffer[i * 2] = sample_r;
                buffer[i * 2 + 1] = sample_l;
            } else {
                buffer[i] = (sample_l + sample_r) / 2.0;
            }
        }

        self.sample_index += 512;
        Some(buffer)
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }
}

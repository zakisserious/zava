pub mod fifo;
pub mod pulse;
pub mod test_signal;

#[cfg(feature = "cpal")]
pub mod cpal_backend;

use crate::config::{Config, InputMethod};
#[cfg(feature = "cpal")]
use self::cpal_backend::CpalBackend;
use self::fifo::FifoBackend;
use self::pulse::PulseBackend;
use self::test_signal::TestSignalBackend;

pub enum AudioSource {
    Pulse(PulseBackend),
    #[cfg(feature = "cpal")]
    Cpal(CpalBackend),
    Fifo(FifoBackend),
    Test(TestSignalBackend),
}

impl AudioSource {
    pub fn new(config: &Config) -> Result<Self, String> {
        let method = config.input.method;
        let source = &config.input.source;
        let rate = config.input.sample_rate;
        let channels = config.input.channels;
        let bits = config.input.sample_bits;

        match method {
            InputMethod::Pipewire | InputMethod::Pulse => {
                match PulseBackend::new(source, rate, channels) {
                    Ok(b) => Ok(AudioSource::Pulse(b)),
                    Err(err) => {
                        #[cfg(feature = "cpal")]
                        {
                            eprintln!("Warning: Pulse/Pipewire error ({err}), trying CPAL fallback...");
                            let cpal = CpalBackend::new(source, rate, channels)
                                .map_err(|e| format!("PulseAudio failed ({err}) and CPAL failed: {e}"))?;
                            Ok(AudioSource::Cpal(cpal))
                        }
                        #[cfg(not(feature = "cpal"))]
                        Err(format!(
                            "PulseAudio failed ({err}) and this build has no CPAL fallback (enable the `cpal` feature)"
                        ))
                    }
                }
            }
            InputMethod::Cpal | InputMethod::Alsa => {
                #[cfg(feature = "cpal")]
                {
                    let cpal = CpalBackend::new(source, rate, channels)
                        .map_err(|e| format!("CPAL initialization failed: {e}"))?;
                    Ok(AudioSource::Cpal(cpal))
                }
                #[cfg(not(feature = "cpal"))]
                Err("CPAL input requires the `cpal` feature (not enabled in this build)".to_string())
            }
            InputMethod::Fifo => {
                let fifo = FifoBackend::new(source, rate, channels, bits)
                    .map_err(|e| format!("FIFO initialization failed: {e}"))?;
                Ok(AudioSource::Fifo(fifo))
            }
            InputMethod::Test => {
                Ok(AudioSource::Test(TestSignalBackend::new(rate, channels)))
            }
        }
    }

    pub fn read_samples(&mut self) -> Option<Vec<f64>> {
        match self {
            AudioSource::Pulse(b) => b.read_samples(),
            #[cfg(feature = "cpal")]
            AudioSource::Cpal(b) => b.read_samples(),
            AudioSource::Fifo(b) => b.read_samples(),
            AudioSource::Test(b) => b.read_samples(),
        }
    }

    pub fn sample_rate(&self) -> u32 {
        match self {
            AudioSource::Pulse(b) => b.sample_rate(),
            #[cfg(feature = "cpal")]
            AudioSource::Cpal(b) => b.sample_rate(),
            AudioSource::Fifo(b) => b.sample_rate(),
            AudioSource::Test(b) => b.sample_rate(),
        }
    }

    pub fn channels(&self) -> u32 {
        match self {
            AudioSource::Pulse(b) => b.channels(),
            #[cfg(feature = "cpal")]
            AudioSource::Cpal(b) => b.channels(),
            AudioSource::Fifo(b) => b.channels(),
            AudioSource::Test(b) => b.channels(),
        }
    }
}

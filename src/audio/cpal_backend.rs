use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, Stream, StreamConfig};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};

pub struct CpalBackend {
    rx: Receiver<Vec<f64>>,
    _stream: Stream,
    sample_rate: u32,
    channels: u32,
}

impl CpalBackend {
    pub fn new(device_name: &str, target_rate: u32, target_channels: u32) -> Result<Self, String> {
        let host = cpal::default_host();
        let device = if device_name.is_empty() || device_name == "auto" {
            host.default_input_device()
                .ok_or_else(|| "No default audio input device found".to_string())?
        } else {
            let mut found = None;
            if let Ok(devices) = host.input_devices() {
                for d in devices {
                    if let Ok(desc) = d.description() {
                        if desc.name().contains(device_name) {
                            found = Some(d);
                            break;
                        }
                    }
                }
            }
            found.ok_or_else(|| format!("Could not find audio device matching '{device_name}'"))?
        };

        let default_config = device
            .default_input_config()
            .map_err(|e| format!("Failed to get default input config: {e}"))?;

        let sample_rate = if target_rate > 0 { target_rate } else { default_config.sample_rate() };
        let channels = if target_channels > 0 { target_channels } else { default_config.channels() as u32 };

        let config = StreamConfig {
            channels: channels as u16,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let (tx, rx): (SyncSender<Vec<f64>>, Receiver<Vec<f64>>) = sync_channel(16);
        let buffer_size = 512 * channels as usize;
        let accumulator = Arc::new(Mutex::new(Vec::with_capacity(buffer_size * 2)));

        let err_fn = |err| eprintln!("CPAL input stream error: {err}");

        let stream = match default_config.sample_format() {
            SampleFormat::F32 => {
                let acc = accumulator.clone();
                let tx = tx.clone();
                device.build_input_stream(
                    config,
                    move |data: &[f32], _: &_| {
                        let mut guard = acc.lock().unwrap();
                        for &s in data {
                            guard.push(s as f64 * 32767.0);
                            if guard.len() >= buffer_size {
                                let chunk: Vec<f64> = guard.drain(0..buffer_size).collect();
                                let _ = tx.try_send(chunk);
                            }
                        }
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::I16 => {
                let acc = accumulator.clone();
                let tx = tx.clone();
                device.build_input_stream(
                    config,
                    move |data: &[i16], _: &_| {
                        let mut guard = acc.lock().unwrap();
                        for &s in data {
                            guard.push(s as f64);
                            if guard.len() >= buffer_size {
                                let chunk: Vec<f64> = guard.drain(0..buffer_size).collect();
                                let _ = tx.try_send(chunk);
                            }
                        }
                    },
                    err_fn,
                    None,
                )
            }
            SampleFormat::U16 => {
                let acc = accumulator.clone();
                let tx = tx.clone();
                device.build_input_stream(
                    config,
                    move |data: &[u16], _: &_| {
                        let mut guard = acc.lock().unwrap();
                        for &s in data {
                            let s16 = (s as i32 - 32768) as f64;
                            guard.push(s16);
                            if guard.len() >= buffer_size {
                                let chunk: Vec<f64> = guard.drain(0..buffer_size).collect();
                                let _ = tx.try_send(chunk);
                            }
                        }
                    },
                    err_fn,
                    None,
                )
            }
            _ => return Err("Unsupported CPAL sample format".to_string()),
        }
        .map_err(|e| format!("Failed to build CPAL input stream: {e}"))?;

        stream.play().map_err(|e| format!("Failed to play CPAL stream: {e}"))?;

        Ok(Self {
            rx,
            _stream: stream,
            sample_rate,
            channels,
        })
    }

    pub fn read_samples(&mut self) -> Option<Vec<f64>> {
        let mut all_samples = Vec::new();
        while let Ok(data) = self.rx.try_recv() {
            all_samples.extend(data);
        }
        if all_samples.is_empty() {
            None
        } else {
            let max_cap = 8192 * self.channels as usize;
            if all_samples.len() > max_cap {
                let start = all_samples.len() - max_cap;
                Some(all_samples[start..].to_vec())
            } else {
                Some(all_samples)
            }
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }
}

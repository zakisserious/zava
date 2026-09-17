use std::fs::OpenOptions;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct FifoBackend {
    rx: Receiver<Vec<f64>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    sample_rate: u32,
    channels: u32,
}

impl FifoBackend {
    pub fn new(path: &str, sample_rate: u32, channels: u32, sample_bits: u32) -> Result<Self, String> {
        let p = Path::new(path);
        if !p.exists() {
            let c_path = std::ffi::CString::new(path)
                .map_err(|e| format!("Invalid FIFO path: {e}"))?;
            unsafe {
                libc::mkfifo(c_path.as_ptr(), 0o666);
            }
        }

        let (tx, rx): (SyncSender<Vec<f64>>, Receiver<Vec<f64>>) = sync_channel(16);
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        let path_buf = path.to_string();
        let bytes_per_sample = (sample_bits / 8).max(1) as usize;
        let samples_per_read = 512 * channels as usize;
        let bytes_to_read = samples_per_read * bytes_per_sample;

        let handle = thread::Builder::new()
            .name("zava-fifo-input".to_string())
            .spawn(move || {
                while running_clone.load(Ordering::Relaxed) {
                    let file_result = OpenOptions::new().read(true).open(&path_buf);
                    if let Ok(mut file) = file_result {
                        let mut raw_buf = vec![0u8; bytes_to_read];
                        let mut sample_buf = vec![0.0f64; samples_per_read];

                        while running_clone.load(Ordering::Relaxed) {
                            match file.read_exact(&mut raw_buf) {
                                Ok(_) => {
                                    for i in 0..samples_per_read {
                                        if bytes_per_sample == 2 {
                                            let s = i16::from_le_bytes([raw_buf[i * 2], raw_buf[i * 2 + 1]]);
                                            sample_buf[i] = s as f64;
                                        } else if bytes_per_sample == 1 {
                                            let s = raw_buf[i] as i8;
                                            sample_buf[i] = (s as f64) * 256.0;
                                        } else if bytes_per_sample >= 4 {
                                            let s = i32::from_le_bytes([
                                                raw_buf[i * 4],
                                                raw_buf[i * 4 + 1],
                                                raw_buf[i * 4 + 2],
                                                raw_buf[i * 4 + 3],
                                            ]);
                                            sample_buf[i] = (s as f64) / 65536.0;
                                        }
                                    }
                                    let _ = tx.try_send(sample_buf.clone());
                                }
                                Err(_) => {
                                    // Pipe closed, wait before trying again
                                    thread::sleep(Duration::from_millis(50));
                                    break;
                                }
                            }
                        }
                    } else {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            })
            .map_err(|e| format!("Failed to spawn FIFO thread: {e}"))?;

        Ok(Self {
            rx,
            running,
            handle: Some(handle),
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

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    pub fn channels(&self) -> u32 {
        self.channels
    }
}

impl Drop for FifoBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

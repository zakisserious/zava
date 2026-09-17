use std::io::{BufReader, Read};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

pub struct PulseBackend {
    rx: Receiver<Vec<f64>>,
    running: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
    sample_rate: u32,
    channels: u32,
}

impl PulseBackend {
    pub fn new(source: &str, sample_rate: u32, channels: u32) -> Result<Self, String> {
        let resolved_source = if source.is_empty() || source == "auto" {
            Self::detect_default_monitor_source().unwrap_or_default()
        } else {
            source.to_string()
        };

        eprintln!(
            "[zava] PulseAudio/PipeWire source: {}",
            if resolved_source.is_empty() {
                "<default>"
            } else {
                &resolved_source
            }
        );

        let (tx, rx): (SyncSender<Vec<f64>>, Receiver<Vec<f64>>) = sync_channel(64);
        let running = Arc::new(AtomicBool::new(true));
        let running_clone = running.clone();

        // Check if `parec` is available on the system before spawning the thread
        if std::process::Command::new("parec").arg("--version").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status().is_err() {
            return Err("`parec` command not found on system, cannot use Pulse/Pipewire backend".to_string());
        }

        let handle = thread::Builder::new()
            .name("zava-pulse-input".to_string())
            .spawn(move || {
                Self::capture_loop(resolved_source, sample_rate, channels, tx, running_clone);
            })
            .map_err(|e| format!("Failed to spawn PulseAudio thread: {e}"))?;

        Ok(Self {
            rx,
            running,
            handle: Some(handle),
            sample_rate,
            channels,
        })
    }

    fn capture_loop(
        source: String,
        sample_rate: u32,
        channels: u32,
        tx: SyncSender<Vec<f64>>,
        running: Arc<AtomicBool>,
    ) {
        while running.load(Ordering::Relaxed) {
            // Use parec (PulseAudio record) which forces PipeWire to activate/unsuspend
            // the monitor source. This is the most reliable method on PipeWire systems.
            let mut cmd = Command::new("parec");
            cmd.args([
                "--format=s16le",
                &format!("--rate={sample_rate}"),
                &format!("--channels={channels}"),
                "--latency-msec=20",
            ]);
            if !source.is_empty() {
                cmd.arg(format!("--device={source}"));
            }
            cmd.stdout(Stdio::piped()).stderr(Stdio::null());

            let mut child: Child = match cmd.spawn() {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[zava] Failed to spawn parec: {e}. Is pulseaudio/pipewire-pulse installed? Retrying in 2s...");
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };

            let stdout: ChildStdout = match child.stdout.take() {
                Some(s) => s,
                None => {
                    eprintln!("[zava] parec: no stdout handle");
                    let _ = child.kill();
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            };

            let mut reader = BufReader::with_capacity(65536, stdout);

            // 256 samples per channel per chunk for ultra-low-latency transfer (~5.8ms at 44.1kHz)
            let samples_per_chunk = 256 * channels as usize;
            let bytes_per_chunk = samples_per_chunk * 2; // s16le = 2 bytes/sample
            let mut byte_buf = vec![0u8; bytes_per_chunk];
            let mut sample_buf = vec![0.0f64; samples_per_chunk];

            'read: while running.load(Ordering::Relaxed) {
                // Fill the full chunk before processing (handles partial reads from parec)
                let mut filled = 0;
                while filled < bytes_per_chunk {
                    match reader.read(&mut byte_buf[filled..]) {
                        Ok(0) => break 'read, // EOF — parec died
                        Ok(n) => filled += n,
                        Err(_) => break 'read,
                    }
                }

                // Convert s16le bytes → f64 samples (range -32768..32767)
                for i in 0..samples_per_chunk {
                    let s16 = i16::from_le_bytes([byte_buf[i * 2], byte_buf[i * 2 + 1]]);
                    sample_buf[i] = s16 as f64;
                }

                // Send sample chunk; if buffer is full, drop oldest or frame to avoid unbounded growth
                let _ = tx.try_send(sample_buf.clone());
            }

            let _ = child.kill();
            let _ = child.wait();

            if running.load(Ordering::Relaxed) {
                eprintln!("[zava] parec exited unexpectedly, restarting in 1s...");
                thread::sleep(Duration::from_secs(1));
            }
        }
    }

    pub fn read_samples(&mut self) -> Option<Vec<f64>> {
        // Collect ALL samples that arrived from parec since last frame
        let mut all_samples = Vec::new();
        while let Ok(data) = self.rx.try_recv() {
            all_samples.extend(data);
        }
        if all_samples.is_empty() {
            None
        } else {
            // Cap at 8192 stereo samples (approx 185ms) if the consumer lagged,
            // preventing latency accumulation while preserving continuous audio
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

    fn detect_default_monitor_source() -> Option<String> {
        // Strategy 1: pactl info → default sink → append .monitor
        if let Ok(output) = Command::new("pactl").arg("info").output() {
            let text = String::from_utf8_lossy(&output.stdout);
            for line in text.lines() {
                let line = line.trim();
                if line.starts_with("Default Sink:") {
                    let sink_name = line.trim_start_matches("Default Sink:").trim();
                    if !sink_name.is_empty() {
                        let monitor = format!("{sink_name}.monitor");
                        eprintln!("[zava] Detected monitor source: {monitor}");
                        return Some(monitor);
                    }
                }
            }
        }

        // Strategy 2: pick first RUNNING monitor, then any monitor
        if let Ok(output) = Command::new("pactl")
            .args(["list", "sources", "short"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut fallback: Option<String> = None;
            for line in text.lines() {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 2 && parts[1].ends_with(".monitor") {
                    if parts.iter().any(|&p| p == "RUNNING") {
                        return Some(parts[1].to_string());
                    }
                    if fallback.is_none() {
                        fallback = Some(parts[1].to_string());
                    }
                }
            }
            return fallback;
        }

        None
    }
}

impl Drop for PulseBackend {
    fn drop(&mut self) {
        self.stop();
    }
}

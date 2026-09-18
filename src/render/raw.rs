use crate::config::{BitFormat, DataFormat, OutputConfig};
use std::fs::OpenOptions;
use std::io::{self, Write};

pub struct RawRenderer {
    writer: Box<dyn Write + Send>,
    data_format: DataFormat,
    bit_format: BitFormat,
    ascii_max_range: u32,
    bar_delimiter: u8,
    frame_delimiter: u8,
}

impl RawRenderer {
    pub fn new(config: &OutputConfig) -> Result<Self, String> {
        let writer: Box<dyn Write + Send> = if config.raw_target == "/dev/stdout" || config.raw_target.is_empty() {
            Box::new(io::stdout())
        } else {
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .open(&config.raw_target)
                .map_err(|e| format!("Failed to open raw target {}: {e}", config.raw_target))?;
            Box::new(file)
        };

        Ok(Self {
            writer,
            data_format: config.data_format,
            bit_format: config.bit_format,
            ascii_max_range: config.ascii_max_range,
            bar_delimiter: config.bar_delimiter,
            frame_delimiter: config.frame_delimiter,
        })
    }

    pub fn render(&mut self, bars: &[f64]) -> io::Result<()> {
        match self.data_format {
            DataFormat::Binary => match self.bit_format {
                BitFormat::Bits8 => {
                    let mut buf = Vec::with_capacity(bars.len());
                    for &val in bars {
                        let b = (val * 255.0).clamp(0.0, 255.0) as u8;
                        buf.push(b);
                    }
                    self.writer.write_all(&buf)?;
                }
                BitFormat::Bits16 => {
                    let mut buf = Vec::with_capacity(bars.len() * 2);
                    for &val in bars {
                        let w = (val * 65535.0).clamp(0.0, 65535.0) as u16;
                        buf.extend_from_slice(&w.to_ne_bytes());
                    }
                    self.writer.write_all(&buf)?;
                }
            },
            DataFormat::Ascii => {
                let mut line = String::new();
                for (i, &val) in bars.iter().enumerate() {
                    let scaled = (val * self.ascii_max_range as f64).clamp(0.0, self.ascii_max_range as f64) as u32;
                    line.push_str(&scaled.to_string());
                    if i + 1 < bars.len() {
                        line.push(self.bar_delimiter as char);
                    }
                }
                line.push(self.frame_delimiter as char);
                self.writer.write_all(line.as_bytes())?;
            }
        }
        self.writer.flush()?;
        Ok(())
    }
}

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_serial::SerialPortBuilderExt;

use crate::error::{Error, Result};

pub struct SerialConnection {
    port: tokio_serial::SerialStream,
    timeout: Duration,
}

impl SerialConnection {
    pub fn open(path: &str, baud_rate: u32, timeout_ms: u64) -> Result<Self> {
        let port = tokio_serial::new(path, baud_rate)
            .timeout(Duration::from_millis(timeout_ms))
            .open_native_async()
            .map_err(|e| Error::Serial(e))?;

        Ok(Self {
            port,
            timeout: Duration::from_millis(timeout_ms),
        })
    }

    pub async fn write_bytes(&mut self, data: &[u8]) -> Result<()> {
        self.port.write_all(data).await?;
        self.port.flush().await?;
        Ok(())
    }

    /// Read until the ELM327 prompt character '>' is received.
    /// Returns the response text (excluding the prompt).
    pub async fn read_until_prompt(&mut self) -> Result<String> {
        let mut buf = Vec::with_capacity(256);
        let mut byte = [0u8; 1];

        let result = tokio::time::timeout(self.timeout, async {
            loop {
                match self.port.read(&mut byte).await {
                    Ok(0) => return Err(Error::Timeout),
                    Ok(_) => {
                        if byte[0] == b'>' {
                            break;
                        }
                        buf.push(byte[0]);
                    }
                    Err(e) => return Err(Error::Io(e)),
                }
            }
            Ok(())
        })
        .await;

        match result {
            Ok(Ok(())) => {
                let response = String::from_utf8_lossy(&buf).trim().to_string();
                Ok(response)
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err(Error::Timeout),
        }
    }

    /// Drain any pending data from the serial buffer.
    pub async fn drain(&mut self) -> Result<()> {
        let mut byte = [0u8; 1];
        loop {
            match tokio::time::timeout(Duration::from_millis(50), self.port.read(&mut byte)).await {
                Ok(Ok(0)) | Err(_) => break,
                Ok(Ok(_)) => continue,
                Ok(Err(_)) => break,
            }
        }
        Ok(())
    }
}

/// List available serial ports and let the user select one.
/// Filters out /dev/tty.* duplicates on macOS, preferring /dev/cu.* for outgoing connections.
/// Returns the selected port's path.
pub fn select_port() -> Result<String> {
    let all_ports = tokio_serial::available_ports().map_err(|e| Error::Serial(e))?;

    // On macOS, each device gets both /dev/tty.* and /dev/cu.* entries.
    // Filter out tty.* when a matching cu.* exists — cu.* is correct for outgoing connections.
    let ports: Vec<_> = all_ports
        .iter()
        .filter(|p| {
            if let Some(tty_suffix) = p.port_name.strip_prefix("/dev/tty.") {
                let cu_name = format!("/dev/cu.{}", tty_suffix);
                !all_ports.iter().any(|other| other.port_name == cu_name)
            } else {
                true
            }
        })
        .collect();

    if ports.is_empty() {
        return Err(Error::Protocol("no serial ports found".into()));
    }

    if ports.len() == 1 {
        let port = &ports[0].port_name;
        println!("Found one device: {} ({})", port, port_type_label(&ports[0].port_type));
        return Ok(port.clone());
    }

    println!("Available serial ports:");
    for (i, port) in ports.iter().enumerate() {
        println!("  [{}] {} ({})", i + 1, port.port_name, port_type_label(&port.port_type));
    }

    loop {
        print!("Select port [1-{}]: ", ports.len());
        std::io::Write::flush(&mut std::io::stdout()).map_err(Error::Io)?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input).map_err(Error::Io)?;

        if let Ok(n) = input.trim().parse::<usize>() {
            if n > 0 && n <= ports.len() {
                return Ok(ports[n - 1].port_name.clone());
            }
        }
        println!("Invalid selection, try again.");
    }
}

fn port_type_label(port_type: &tokio_serial::SerialPortType) -> &'static str {
    match port_type {
        tokio_serial::SerialPortType::UsbPort(_) => "USB",
        tokio_serial::SerialPortType::BluetoothPort => "Bluetooth",
        tokio_serial::SerialPortType::PciPort => "PCI",
        tokio_serial::SerialPortType::Unknown => "Unknown",
    }
}

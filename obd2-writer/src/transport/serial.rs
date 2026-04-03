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

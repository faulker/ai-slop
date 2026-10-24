//! GPU readback of a wgpu texture into tightly packed RGBA8 frames.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const RING_SIZE: usize = 3;

/// Bytes per texture row rounded up to wgpu's copy alignment.
pub fn padded_bytes_per_row(width: u32) -> u32 {
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    (width * 4).div_ceil(align) * align
}

/// Copies rows out of a padded readback buffer into a tightly packed RGBA8 vector.
pub fn strip_row_padding(
    data: &[u8],
    width: u32,
    height: u32,
    padded_bytes_per_row: u32,
) -> Vec<u8> {
    let row = (width * 4) as usize;
    let stride = padded_bytes_per_row as usize;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    out
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SlotState {
    Free,
    /// Copy recorded, waiting for `after_submit`.
    Copied,
    /// `map_async` issued, waiting for the callback.
    Mapping,
}

struct Slot {
    buffer: wgpu::Buffer,
    state: SlotState,
    ready: Arc<AtomicBool>,
}

/// Ring of MAP_READ buffers used to read a texture back to the CPU without stalling.
pub struct FrameCapture {
    slots: Vec<Slot>,
    width: u32,
    height: u32,
    padded: u32,
    /// Slot indices in the order their copies were submitted.
    pending: std::collections::VecDeque<usize>,
}

impl FrameCapture {
    /// Allocates the readback ring for a `width` x `height` RGBA8 texture.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> FrameCapture {
        let padded = padded_bytes_per_row(width);
        let slots = (0..RING_SIZE)
            .map(|i| Slot {
                buffer: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&format!("cast-readback-{i}")),
                    size: padded as u64 * height as u64,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: SlotState::Free,
                ready: Arc::new(AtomicBool::new(false)),
            })
            .collect();
        FrameCapture {
            slots,
            width,
            height,
            padded,
            pending: std::collections::VecDeque::new(),
        }
    }

    /// Returns the capture size in pixels.
    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Records a copy of `texture` into the next free ring buffer; false if every buffer is busy.
    pub fn copy(&mut self, encoder: &mut wgpu::CommandEncoder, texture: &wgpu::Texture) -> bool {
        let Some(idx) = self.slots.iter().position(|s| s.state == SlotState::Free) else {
            return false;
        };
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &self.slots[idx].buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );
        self.slots[idx].state = SlotState::Copied;
        true
    }

    /// Issues `map_async` for buffers copied this frame; call right after `queue.submit`.
    pub fn after_submit(&mut self) {
        for (idx, slot) in self.slots.iter_mut().enumerate() {
            if slot.state != SlotState::Copied {
                continue;
            }
            slot.ready.store(false, Ordering::SeqCst);
            let ready = slot.ready.clone();
            slot.buffer
                .slice(..)
                .map_async(wgpu::MapMode::Read, move |res| match res {
                    Ok(()) => ready.store(true, Ordering::SeqCst),
                    Err(e) => log::error!("frame readback map failed: {e}"),
                });
            slot.state = SlotState::Mapping;
            self.pending.push_back(idx);
        }
    }

    /// Polls the device without blocking; returns the oldest finished frame as tight RGBA bytes.
    pub fn poll(&mut self, device: &wgpu::Device) -> Option<Vec<u8>> {
        if let Err(e) = device.poll(wgpu::PollType::Poll) {
            log::warn!("device poll failed: {e}");
        }
        let idx = *self.pending.front()?;
        let slot = &mut self.slots[idx];
        if !slot.ready.load(Ordering::SeqCst) {
            return None;
        }
        self.pending.pop_front();
        let frame = {
            let view = slot.buffer.slice(..).get_mapped_range();
            strip_row_padding(&view, self.width, self.height, self.padded)
        };
        slot.buffer.unmap();
        slot.ready.store(false, Ordering::SeqCst);
        slot.state = SlotState::Free;
        Some(frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Row stride is aligned to 256 and never smaller than the tight row.
    #[test]
    fn padded_row_alignment() {
        assert_eq!(padded_bytes_per_row(1), 256);
        assert_eq!(padded_bytes_per_row(64), 256);
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1280), 5120);
        assert_eq!(padded_bytes_per_row(1920), 7680);
        for w in 1..600 {
            let p = padded_bytes_per_row(w);
            assert_eq!(p % 256, 0);
            assert!(p >= w * 4);
        }
    }

    /// 3x2 image: padded rows of 256 bytes shrink to 12 bytes each.
    #[test]
    fn strip_padding_3x2() {
        let padded = padded_bytes_per_row(3) as usize;
        let mut data = vec![0xEEu8; padded * 2];
        for y in 0..2 {
            for i in 0..12 {
                data[y * padded + i] = (y * 12 + i) as u8;
            }
        }
        let out = strip_row_padding(&data, 3, 2, padded as u32);
        assert_eq!(out, (0..24).collect::<Vec<u8>>());
    }

    /// 65x3 image crosses the 256 byte boundary so the stride is 512.
    #[test]
    fn strip_padding_65x3() {
        let padded = padded_bytes_per_row(65);
        assert_eq!(padded, 512);
        let mut data = vec![0xEEu8; padded as usize * 3];
        for y in 0..3usize {
            for i in 0..260usize {
                data[y * 512 + i] = ((y * 7 + i) % 251) as u8;
            }
        }
        let out = strip_row_padding(&data, 65, 3, padded);
        assert_eq!(out.len(), 65 * 4 * 3);
        for y in 0..3usize {
            for i in 0..260usize {
                assert_eq!(out[y * 260 + i], ((y * 7 + i) % 251) as u8);
            }
        }
    }
}

//! `lora-phy` always logs through `defmt`, which needs a global logger. This
//! one discards everything so the serial port stays plain text for
//! `espflash monitor`.

#[defmt::global_logger]
struct Discard;

unsafe impl defmt::Logger for Discard {
    fn acquire() {}
    unsafe fn flush() {}
    unsafe fn release() {}
    unsafe fn write(_bytes: &[u8]) {}
}

//! BMW Motorrad support, starting with the 2023 R 1250 GS Adventure (model code K51).
//! Modules are reached over D-CAN with extended addressing ("6F1:<module>"); see
//! `docs/bmw-k51/` for what has and hasn't been confirmed on the bike.

pub mod explain;
pub mod modules;
pub mod settings;

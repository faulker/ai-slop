//! Key derivation and per-chunk AEAD.
//!
//! One shared passphrase is the only common secret, so the Argon2id salt is
//! a fixed application constant (documented tradeoff: no per-user salts,
//! anyone with the passphrase decrypts everything; see docs/BUILD-PLAN.md).
//! Every chunk is sealed independently with XChaCha20-Poly1305, the 11-byte
//! chunk header as associated data, and a nonce derived from
//! `message_id || chunk_index` — never transmitted.

use crate::AetrError;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};

/// Fixed KDF salt; versioned so a future protocol revision can rotate it.
/// The plan called for "aetr-v1", but the argon2 crate enforces a minimum
/// salt of 8 bytes, so the constant carries a "-salt" suffix.
pub const KDF_SALT: &[u8] = b"aetr-v1-salt";
/// Argon2id memory cost in KiB (OWASP baseline, sized for low-end Android).
pub const KDF_M_COST_KIB: u32 = 19456;
/// Argon2id iteration count.
pub const KDF_T_COST: u32 = 2;
/// Argon2id parallelism.
pub const KDF_P_COST: u32 = 1;
/// Poly1305 tag length appended to every chunk ciphertext.
pub const TAG_LEN: usize = 16;

/// Derives the 32-byte session key from the shared passphrase using
/// Argon2id (m=19456 KiB, t=2, p=1, salt "aetr-v1"). Blocking (~100 ms).
pub fn derive_key(passphrase: &str) -> Result<[u8; 32], AetrError> {
    let params = Params::new(KDF_M_COST_KIB, KDF_T_COST, KDF_P_COST, Some(32))
        .map_err(|e| AetrError::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(passphrase.as_bytes(), KDF_SALT, &mut key)
        .map_err(|e| AetrError::Kdf(e.to_string()))?;
    Ok(key)
}

/// Builds the 24-byte XChaCha20 nonce for a chunk:
/// `message_id (8 bytes LE) || chunk_index (1 byte) || 15 zero bytes`.
pub fn nonce_for(message_id: u64, chunk_index: u8) -> [u8; 24] {
    let mut nonce = [0u8; 24];
    nonce[..8].copy_from_slice(&message_id.to_le_bytes());
    nonce[8] = chunk_index;
    nonce
}

/// Seals one chunk: encrypts `plaintext` bound to the 11-byte header (AAD).
/// Output is ciphertext plus the 16-byte Poly1305 tag.
pub fn seal_chunk(
    key: &[u8; 32],
    header: &[u8; crate::frame::HEADER_LEN],
    message_id: u64,
    chunk_index: u8,
    plaintext: &[u8],
) -> Result<Vec<u8>, AetrError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = nonce_for(message_id, chunk_index);
    cipher
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload { msg: plaintext, aad: header },
        )
        .map_err(|_| AetrError::Kdf("encryption failed".into()))
}

/// Opens one chunk. Fails with `AuthFailed` when the key, nonce inputs, or
/// header do not match what was sealed — a wrong passphrase hits this on
/// every chunk.
pub fn open_chunk(
    key: &[u8; 32],
    header: &[u8; crate::frame::HEADER_LEN],
    message_id: u64,
    chunk_index: u8,
    ciphertext: &[u8],
) -> Result<Vec<u8>, AetrError> {
    let cipher = XChaCha20Poly1305::new(key.into());
    let nonce = nonce_for(message_id, chunk_index);
    cipher
        .decrypt(
            XNonce::from_slice(&nonce),
            Payload { msg: ciphertext, aad: header },
        )
        .map_err(|_| AetrError::AuthFailed)
}

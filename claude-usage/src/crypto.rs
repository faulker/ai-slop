use crate::error::{AppError, Result};
use aes::Aes128;
use cbc::cipher::{BlockDecryptMut, KeyIvInit};
use pbkdf2::pbkdf2_hmac;
use sha1::Sha1;
use sha2::{Digest, Sha256};

type Aes128CbcDec = cbc::Decryptor<Aes128>;

const SALT: &[u8] = b"saltysalt";
const ITERATIONS: u32 = 1003;
const KEY_LEN: usize = 16;
const IV: [u8; 16] = [0x20; 16];

pub fn derive_key(password: &str) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2_hmac::<Sha1>(password.as_bytes(), SALT, ITERATIONS, &mut key);
    key
}

pub fn decrypt_cookie(
    encrypted: &[u8],
    host_key: &str,
    db_version: i32,
    key: &[u8; KEY_LEN],
) -> Result<String> {
    // Must start with "v10" prefix (3 bytes)
    if encrypted.len() < 3 || &encrypted[..3] != b"v10" {
        return Err(AppError::Decrypt {
            msg: "missing v10 prefix".into(),
        });
    }

    let ciphertext = &encrypted[3..];

    // AES block size is 16; ciphertext must be a multiple of 16
    if ciphertext.is_empty() || ciphertext.len() % 16 != 0 {
        return Err(AppError::Decrypt {
            msg: format!(
                "invalid ciphertext length: {} (must be non-zero multiple of 16)",
                ciphertext.len()
            ),
        });
    }

    // Decrypt in place on a copy
    let mut buf = ciphertext.to_vec();
    let decryptor = Aes128CbcDec::new(key.into(), &IV.into());
    let decrypted = decryptor
        .decrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut buf)
        .map_err(|e| AppError::Decrypt {
            msg: format!("AES decryption failed: {e}"),
        })?;

    // DB version >= 24 prepends a 32-byte SHA256 hash of the host_key
    let plaintext = if db_version >= 24 {
        if decrypted.len() < 32 {
            return Err(AppError::Decrypt {
                msg: format!(
                    "decrypted value too short for v24 hash prefix: {} bytes",
                    decrypted.len()
                ),
            });
        }
        let stored_hash = &decrypted[..32];
        let expected_hash = Sha256::digest(host_key.as_bytes());
        if stored_hash != expected_hash.as_slice() {
            return Err(AppError::Decrypt {
                msg: "SHA256 host_key hash mismatch".into(),
            });
        }
        &decrypted[32..]
    } else {
        decrypted
    };

    String::from_utf8(plaintext.to_vec()).map_err(|e| AppError::Decrypt {
        msg: format!("decrypted value is not valid UTF-8: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use aes::Aes128;
    use cbc::cipher::{BlockEncryptMut, KeyIvInit};

    type Aes128CbcEnc = cbc::Encryptor<Aes128>;

    fn encrypt_value(plaintext: &[u8], key: &[u8; KEY_LEN]) -> Vec<u8> {
        // Allocate buffer with room for padding (up to one extra block)
        let mut buf = vec![0u8; plaintext.len() + 16];
        buf[..plaintext.len()].copy_from_slice(plaintext);
        let encryptor = Aes128CbcEnc::new(key.into(), &IV.into());
        let ciphertext = encryptor
            .encrypt_padded_mut::<cbc::cipher::block_padding::Pkcs7>(&mut buf, plaintext.len())
            .unwrap();
        let mut result = b"v10".to_vec();
        result.extend_from_slice(ciphertext);
        result
    }

    #[test]
    fn derive_key_produces_16_bytes() {
        let key = derive_key("test_password");
        assert_eq!(key.len(), KEY_LEN);
    }

    #[test]
    fn derive_key_is_deterministic() {
        let k1 = derive_key("hello");
        let k2 = derive_key("hello");
        assert_eq!(k1, k2);
    }

    #[test]
    fn derive_key_differs_for_different_passwords() {
        let k1 = derive_key("password_a");
        let k2 = derive_key("password_b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn rejects_non_v10_prefix() {
        let key = derive_key("pw");
        let err = decrypt_cookie(b"v11xxxx", "host", 23, &key);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("v10"));
    }

    #[test]
    fn rejects_bad_ciphertext_length() {
        let key = derive_key("pw");
        // v10 + 5 bytes (not a multiple of 16)
        let mut data = b"v10".to_vec();
        data.extend_from_slice(&[0u8; 5]);
        let err = decrypt_cookie(&data, "host", 23, &key);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("length"));
    }

    #[test]
    fn roundtrip_pre_v24() {
        let key = derive_key("my_password");
        let plaintext = b"sk-ant-secret-value-12345";
        let encrypted = encrypt_value(plaintext, &key);
        let decrypted = decrypt_cookie(&encrypted, ".claude.ai", 23, &key).unwrap();
        assert_eq!(decrypted, "sk-ant-secret-value-12345");
    }

    #[test]
    fn roundtrip_v24_with_hash() {
        let key = derive_key("my_password");
        let host = ".claude.ai";
        let host_hash = sha2::Sha256::digest(host.as_bytes());
        let mut plaintext_with_hash = host_hash.to_vec();
        plaintext_with_hash.extend_from_slice(b"cookie-value-abc");
        let encrypted = encrypt_value(&plaintext_with_hash, &key);
        let decrypted = decrypt_cookie(&encrypted, host, 24, &key).unwrap();
        assert_eq!(decrypted, "cookie-value-abc");
    }

    #[test]
    fn v24_hash_mismatch_is_error() {
        let key = derive_key("my_password");
        // Encrypt with hash of "wrong.host"
        let wrong_hash = sha2::Sha256::digest(b"wrong.host");
        let mut plaintext = wrong_hash.to_vec();
        plaintext.extend_from_slice(b"value");
        let encrypted = encrypt_value(&plaintext, &key);
        let err = decrypt_cookie(&encrypted, ".claude.ai", 24, &key);
        assert!(err.is_err());
        assert!(err.unwrap_err().to_string().contains("hash mismatch"));
    }
}

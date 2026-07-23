//! Reed-Solomon erasure coding across text chunks.
//!
//! Text messages of K source chunks get a parity pool of K shards encoded
//! upfront (N = 2K, MDS: any K of 2K recovers). Only the first
//! R = ceil(0.25 K) parity shards are transmitted; the rest stay cached on
//! the sender for the optional receiver-initiated ARQ round. Voice is never
//! erasure-coded (it degrades gracefully per chunk).

use crate::AetrError;
use reed_solomon_erasure::galois_8::ReedSolomon;

/// Number of parity shards transmitted upfront for K source shards.
pub fn repair_count(k: usize) -> usize {
    k.div_ceil(4)
}

/// Total frames transmitted upfront for a K-chunk text message
/// (source + initial repair). Single-chunk messages carry no repair.
pub fn transmit_count(k: usize) -> usize {
    if k <= 1 {
        1
    } else {
        k + repair_count(k)
    }
}

/// Inverts `transmit_count`: recovers K from the header's chunk_count.
/// Returns None for totals no sender produces.
pub fn source_count_from_total(total: usize) -> Option<usize> {
    if total == 1 {
        return Some(1);
    }
    (2..=total).find(|&k| transmit_count(k) == total)
}

/// Encodes the full parity pool: returns K parity shards for K equal-length
/// source shards (pool total N = 2K).
pub fn parity_pool(source: &[Vec<u8>]) -> Result<Vec<Vec<u8>>, AetrError> {
    let k = source.len();
    if k < 2 {
        return Err(AetrError::Fec("parity pool needs at least 2 shards".into()));
    }
    let shard_len = source[0].len();
    if source.iter().any(|s| s.len() != shard_len) || shard_len == 0 {
        return Err(AetrError::Fec("source shards must be equal non-zero length".into()));
    }
    let rs = ReedSolomon::new(k, k).map_err(|e| AetrError::Fec(format!("{e:?}")))?;
    let mut shards: Vec<Vec<u8>> = source.to_vec();
    shards.extend(std::iter::repeat_with(|| vec![0u8; shard_len]).take(k));
    rs.encode(&mut shards).map_err(|e| AetrError::Fec(format!("{e:?}")))?;
    Ok(shards.split_off(k))
}

/// Reconstructs the K source shards from any K present shards out of the
/// 2K pool. `shards` must have length 2K; present entries are Some. On
/// success the first K entries are all Some.
pub fn reconstruct_source(k: usize, shards: &mut [Option<Vec<u8>>]) -> Result<(), AetrError> {
    if shards.len() != 2 * k || k < 2 {
        return Err(AetrError::Fec(format!(
            "reconstruct wants 2K={} slots, got {}",
            2 * k,
            shards.len()
        )));
    }
    let rs = ReedSolomon::new(k, k).map_err(|e| AetrError::Fec(format!("{e:?}")))?;
    rs.reconstruct_data(shards)
        .map_err(|e| AetrError::Fec(format!("{e:?}")))
}

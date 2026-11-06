//! Blob side-channel: large payloads (frames, trees, traces) travel as
//! length-prefixed postcard + LZ4 blobs so command latency stays independent
//! of payload size (architecture §7 + §15.6).

use crate::GwpError;
use crate::schema::MAX_BLOB_BYTES;

/// Compress a blob for the side channel (LZ4, size prepended by lz4_flex).
pub fn encode_blob(data: &[u8]) -> Vec<u8> {
    lz4_flex::compress_prepend_size(data)
}

/// Decompress a side-channel blob with the [`MAX_BLOB_BYTES`] guard.
pub fn decode_blob(blob: &[u8]) -> Result<Vec<u8>, GwpError> {
    let data = lz4_flex::decompress_size_prepended(blob)
        .map_err(|e| GwpError::Blob(format!("lz4: {e}")))?;
    if data.len() > MAX_BLOB_BYTES {
        return Err(GwpError::Blob(format!(
            "decompressed blob of {} bytes exceeds the guard",
            data.len()
        )));
    }
    Ok(data)
}

/// Is compression worthwhile for this payload? (§15.6: LZ4 only for
/// blobs > 4 KiB.)
pub const COMPRESSION_THRESHOLD: usize = 4 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_roundtrip_incompressible() {
        // Pseudo-random bytes: LZ4 can't shrink it, must still round-trip.
        let data: Vec<u8> = (0..64_000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 16) as u8)
            .collect();
        let blob = encode_blob(&data);
        let back = decode_blob(&blob).unwrap();
        assert_eq!(back, data);
    }

    #[test]
    fn blob_compresses_repetitive_data() {
        let data = vec![0xAAu8; COMPRESSION_THRESHOLD * 4];
        let blob = encode_blob(&data);
        assert!(
            blob.len() < data.len() / 10,
            "repetitive data must compress hard: {} -> {}",
            data.len(),
            blob.len()
        );
        assert_eq!(decode_blob(&blob).unwrap(), data);
    }

    #[test]
    fn corrupt_blob_errors_cleanly() {
        let blob = encode_blob(b"hello world");
        let mut corrupt = blob.clone();
        let mid = corrupt.len() / 2;
        corrupt[mid] ^= 0xFF;
        // Corrupting the compressed stream may error or produce garbage;
        // either way it must not panic.
        let _ = decode_blob(&corrupt);
    }

    #[test]
    fn empty_blob_roundtrips() {
        let blob = encode_blob(&[]);
        assert_eq!(decode_blob(&blob).unwrap(), Vec::<u8>::new());
    }
}

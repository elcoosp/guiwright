//! GWP framing: 4-byte little-endian length prefix + postcard-encoded body
//! (architecture §7 "Framing"). Async over any tokio stream; a single
//! `MAX_FRAME_BYTES` guard rejects corrupt or hostile prefixes.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::GwpError;
use crate::schema::MAX_FRAME_BYTES;

/// Encode one frame into a fresh buffer (used by tests and the blob channel).
pub fn encode_frame<T: serde::Serialize>(payload: &T) -> Result<Vec<u8>, GwpError> {
    let body = postcard::to_allocvec(payload)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(GwpError::FrameTooLarge(body.len()));
    }
    let mut out = Vec::with_capacity(4 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    Ok(out)
}

/// Write one length-prefixed frame.
pub async fn write_frame<T: serde::Serialize, W: AsyncWrite + Unpin>(
    sink: &mut W,
    payload: &T,
) -> Result<(), GwpError> {
    let frame = encode_frame(payload)?;
    sink.write_all(&frame).await?;
    sink.flush().await?;
    Ok(())
}

/// Read exactly one length-prefixed frame and decode it.
pub async fn read_frame<T: serde::de::DeserializeOwned, R: AsyncRead + Unpin>(
    stream: &mut R,
) -> Result<T, GwpError> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(GwpError::Io)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_FRAME_BYTES {
        return Err(GwpError::FrameTooLarge(len));
    }
    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await.map_err(GwpError::Io)?;
    Ok(postcard::from_bytes(&body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{GwpRequest, RequestV1};
    use tokio::io::duplex;

    #[tokio::test]
    async fn frames_survive_roundtrip_and_coalescing() {
        let payloads: Vec<GwpRequest> = vec![
            GwpRequest::V1(RequestV1::Ping),
            GwpRequest::V1(RequestV1::Hello {
                proto_version: 1,
                agent: "test".into(),
            }),
            GwpRequest::V1(RequestV1::Bye),
        ];
        let (mut tx, mut rx) = duplex(64);
        // Write all frames back-to-back (coalesced in the pipe).
        for p in &payloads {
            write_frame(&mut tx, p).await.unwrap();
        }
        drop(tx);

        for expected in &payloads {
            let got: GwpRequest = read_frame(&mut rx).await.unwrap();
            assert_eq!(&got, expected);
        }
        // EOF cleanly.
        let res: Result<GwpRequest, _> = read_frame(&mut rx).await;
        assert!(res.is_err());
    }

    #[tokio::test]
    async fn oversized_length_prefix_is_rejected() {
        let (mut tx, mut rx) = duplex(64);
        let evil = (MAX_FRAME_BYTES as u32 + 1).to_le_bytes();
        tx.write_all(&evil).await.unwrap();
        let res: Result<GwpRequest, _> = read_frame(&mut rx).await;
        assert!(matches!(res, Err(GwpError::FrameTooLarge(_))));
    }

    #[test]
    fn encode_frame_prefixes_little_endian_length() {
        let req = GwpRequest::V1(RequestV1::Ping);
        let frame = encode_frame(&req).unwrap();
        let len = u32::from_le_bytes(frame[0..4].try_into().unwrap()) as usize;
        assert_eq!(frame.len(), 4 + len);
    }
}

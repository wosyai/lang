//! Bounded Serde frames delimited by the shared Tokio codec.
use bytes::BytesMut;
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Write};
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::codec::length_delimited::Builder;
use tokio_util::codec::{Decoder, Encoder, LengthDelimitedCodec, LengthDelimitedCodecError};

const HEADER_LEN: usize = 4;
pub const MAX_SESSION_FRAME_BYTES: usize = 1 << 20;

fn frame_codec() -> Builder {
    let mut builder = LengthDelimitedCodec::builder();
    builder.big_endian().length_field_type::<u32>().max_frame_length(MAX_SESSION_FRAME_BYTES);
    builder
}

#[derive(Debug)]
pub enum SessionFrameError {
    Io(io::Error),
    Decode(serde_json::Error),
    Encode(serde_json::Error),
    Empty,
    TooLarge,
}

impl std::fmt::Display for SessionFrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "diagnostic frame I/O: {error}"),
            Self::Decode(error) => write!(f, "invalid diagnostic frame: {error}"),
            Self::Encode(error) => write!(f, "could not encode diagnostic frame: {error}"),
            Self::Empty => f.write_str("empty diagnostic frame"),
            Self::TooLarge => f.write_str("diagnostic frame exceeds its payload limit"),
        }
    }
}

impl std::error::Error for SessionFrameError {}

impl From<io::Error> for SessionFrameError {
    fn from(error: io::Error) -> Self { Self::Io(error) }
}

fn codec_error(error: io::Error) -> SessionFrameError {
    if error.get_ref().is_some_and(|source| source.is::<LengthDelimitedCodecError>()) {
        SessionFrameError::TooLarge
    } else {
        error.into()
    }
}

/// Encodes a complete frame before any bytes reach the transport. The four
/// byte unsigned big-endian length excludes the prefix itself.
pub fn encode_session_frame<T: Serialize>(message: &T) -> Result<BytesMut, SessionFrameError> {
    #[derive(Default)]
    struct FramePayload { bytes: BytesMut, too_large: bool }

    impl Write for FramePayload {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if input.len() > MAX_SESSION_FRAME_BYTES - self.bytes.len() {
                self.too_large = true;
                return Err(io::Error::other("diagnostic frame bound"));
            }
            self.bytes.extend_from_slice(input);
            Ok(input.len())
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    let mut payload = FramePayload::default();
    if let Err(error) = serde_json::to_writer(&mut payload, message) {
        return Err(if payload.too_large { SessionFrameError::TooLarge } else { SessionFrameError::Encode(error) });
    }
    if payload.bytes.is_empty() { return Err(SessionFrameError::Empty); }
    let mut complete = BytesMut::new();
    frame_codec().new_codec().encode(payload.bytes.as_ref(), &mut complete).map_err(codec_error)?;
    Ok(complete)
}

/// The retained decoder and input buffer resume across a canceled read.
pub struct SessionFrameReader {
    codec: LengthDelimitedCodec,
    input: BytesMut,
}

impl Default for SessionFrameReader {
    fn default() -> Self {
        // Keep the prefix in an incomplete frame. EOF just after a valid
        // prefix is still truncation, not the end of an empty stream.
        let codec = frame_codec().length_adjustment(HEADER_LEN as isize).num_skip(0).new_codec();
        Self { codec, input: BytesMut::new() }
    }
}

impl SessionFrameReader {
    pub async fn read<R: AsyncRead + Unpin, T: DeserializeOwned>(
        &mut self, input: &mut R,
    ) -> Result<Option<T>, SessionFrameError> {
        loop {
            if let Some(frame) = self.codec.decode(&mut self.input).map_err(codec_error)? {
                let payload = &frame[HEADER_LEN..];
                if payload.is_empty() { return Err(SessionFrameError::Empty); }
                return serde_json::from_slice(payload).map(Some).map_err(SessionFrameError::Decode);
            }
            let remaining = MAX_SESSION_FRAME_BYTES + HEADER_LEN - self.input.len();
            let received = (&mut *input).take(remaining as u64).read_buf(&mut self.input).await?;
            if received == 0 {
                return if self.input.is_empty() {
                    Ok(None)
                } else {
                    Err(io::Error::new(io::ErrorKind::UnexpectedEof, "truncated diagnostic frame").into())
                };
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BuildSessionCommand, BuildSessionCommandId, BuildSessionReply, BuildSessionRequest, BuildSessionResponse, NodeId};
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    use tokio::io::{duplex, AsyncWriteExt};

    #[test]
    fn legacy_start_and_started_wire_bytes() {
        let id = BuildSessionCommandId(7);
        let command = BuildSessionCommand { id, request: BuildSessionRequest::Start { version: 1 } };
        let response = BuildSessionResponse::<()> {
            id, reply: BuildSessionReply::Started { version: 1, source_root: NodeId([0; 32]) },
        };
        let start = br#"{"id":7,"request":{"kind":"start","version":1}}"#;
        let started = br#"{"id":7,"reply":{"kind":"started","version":1,"source_root":[0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0]}}"#;
        for (encoded, payload) in [(encode_session_frame(&command), &start[..]), (encode_session_frame(&response), &started[..])] {
            let mut expected = (payload.len() as u32).to_be_bytes().to_vec();
            expected.extend_from_slice(payload);
            assert_eq!(encoded.unwrap().as_ref(), expected);
        }
    }

    #[test]
    fn bound_prefix_body_eof_and_cancellation() {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let mut frames = SessionFrameReader::default();
            let (mut tx, mut rx) = duplex(256);
            let first = encode_session_frame(&"first").unwrap();
            let second = encode_session_frame(&"second").unwrap();
            assert_eq!(first.as_ref(), b"\x00\x00\x00\x07\"first\"");
            for partial in [0..2, 2..6] {
                tx.write_all(&first[partial]).await.unwrap();
                let mut interrupted = Box::pin(frames.read::<_, String>(&mut rx));
                assert!(poll_fn(|cx| Poll::Ready(interrupted.as_mut().poll(cx).is_pending())).await);
            }
            tx.write_all(&first[6..]).await.unwrap();
            tx.write_all(&second).await.unwrap();
            drop(tx);
            assert_eq!(frames.read::<_, String>(&mut rx).await.unwrap().as_deref(), Some("first"));
            assert_eq!(frames.read::<_, String>(&mut rx).await.unwrap().as_deref(), Some("second"));
            assert!(frames.read::<_, String>(&mut rx).await.unwrap().is_none());

            let exact = "a".repeat(MAX_SESSION_FRAME_BYTES - 2);
            assert_eq!(encode_session_frame(&exact).unwrap().len(), MAX_SESSION_FRAME_BYTES + HEADER_LEN);
            assert!(matches!(encode_session_frame(&(exact + "b")), Err(SessionFrameError::TooLarge)));
            let mut over = &((MAX_SESSION_FRAME_BYTES as u32) + 1).to_be_bytes()[..];
            assert!(matches!(frames.read::<_, String>(&mut over).await, Err(SessionFrameError::TooLarge)));
            assert_eq!(frames.input.len(), HEADER_LEN);
            for tail in [2, HEADER_LEN, first.len() - 1] {
                let mut incomplete = &first[..tail];
                let err = SessionFrameReader::default().read::<_, String>(&mut incomplete).await.unwrap_err();
                assert!(matches!(err, SessionFrameError::Io(io) if io.kind() == io::ErrorKind::UnexpectedEof));
            }
            let mut empty = &[0; HEADER_LEN][..];
            assert!(matches!(SessionFrameReader::default().read::<_, String>(&mut empty).await,
                Err(SessionFrameError::Empty)));
        });
    }
}

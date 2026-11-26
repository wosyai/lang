//! Bounded length-delimited Serde frames for the local diagnostic protocol.
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Write};
use tokio::io::{AsyncRead, AsyncReadExt};

const HEADER_LEN: usize = 4;
pub const MAX_SESSION_FRAME_BYTES: usize = 1 << 20;

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

/// Encodes a complete frame before any bytes reach the transport. The four
/// byte unsigned big-endian length excludes the prefix itself.
pub fn encode_session_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, SessionFrameError> {
    struct FrameBuffer { bytes: Vec<u8>, too_large: bool }

    impl Write for FrameBuffer {
        fn write(&mut self, input: &[u8]) -> io::Result<usize> {
            if input.len() > MAX_SESSION_FRAME_BYTES + HEADER_LEN - self.bytes.len() {
                self.too_large = true;
                return Err(io::Error::other("diagnostic frame bound"));
            }
            self.bytes.extend_from_slice(input);
            Ok(input.len())
        }

        fn flush(&mut self) -> io::Result<()> { Ok(()) }
    }

    let mut buffer = FrameBuffer { bytes: vec![0; HEADER_LEN], too_large: false };
    if let Err(error) = serde_json::to_writer(&mut buffer, message) {
        return Err(if buffer.too_large { SessionFrameError::TooLarge } else { SessionFrameError::Encode(error) });
    }
    let len = (buffer.bytes.len() - HEADER_LEN) as u32;
    if len == 0 { return Err(SessionFrameError::Empty); }
    buffer.bytes[..HEADER_LEN].copy_from_slice(&len.to_be_bytes());
    Ok(buffer.bytes)
}

/// Resumable decoder: a canceled wait does not lose a prefix or body
/// fragment. Clean EOF is distinct from EOF inside an accepted frame.
#[derive(Default)]
pub struct SessionFrameReader {
    header: [u8; HEADER_LEN],
    header_used: usize,
    payload: Vec<u8>,
    payload_used: usize,
}

impl SessionFrameReader {
    pub async fn read<R: AsyncRead + Unpin, T: DeserializeOwned>(
        &mut self, input: &mut R,
    ) -> Result<Option<T>, SessionFrameError> {
        while self.header_used < HEADER_LEN {
            let received = input.read(&mut self.header[self.header_used..]).await?;
            if received == 0 {
                return if self.header_used == 0 { Ok(None) } else { Err(truncated()) };
            }
            self.header_used += received;
        }
        if self.payload.is_empty() {
            let len = u32::from_be_bytes(self.header) as usize;
            if len == 0 { return Err(SessionFrameError::Empty); }
            if len > MAX_SESSION_FRAME_BYTES { return Err(SessionFrameError::TooLarge); }
            self.payload.resize(len, 0);
        }
        while self.payload_used < self.payload.len() {
            let received = input.read(&mut self.payload[self.payload_used..]).await?;
            if received == 0 { return Err(truncated()); }
            self.payload_used += received;
        }
        let result = serde_json::from_slice(&self.payload).map_err(SessionFrameError::Decode);
        self.header_used = 0;
        self.payload_used = 0;
        self.payload.clear();
        result.map(Some)
    }
}

fn truncated() -> SessionFrameError {
    io::Error::new(io::ErrorKind::UnexpectedEof, "truncated diagnostic frame").into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::future::{poll_fn, Future};
    use std::task::Poll;
    use tokio::io::{duplex, AsyncWriteExt};

    #[test]
    fn bound_prefix_body_eof_and_cancellation() {
        tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(async {
            let mut frames = SessionFrameReader::default();
            let (mut tx, mut rx) = duplex(256);
            let first = encode_session_frame(&"first").unwrap();
            let second = encode_session_frame(&"second").unwrap();
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
            assert!(frames.payload.is_empty());
            let mut incomplete = &first[..first.len() - 1];
            let err = SessionFrameReader::default().read::<_, String>(&mut incomplete).await.unwrap_err();
            assert!(matches!(err, SessionFrameError::Io(io) if io.kind() == io::ErrorKind::UnexpectedEof));
        });
    }
}

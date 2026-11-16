use std::fmt;
use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

pub const MAX_FRAME_BYTES: usize = 1024 * 1024;
pub const MAX_CHUNK_BYTES: usize = 65536;
pub const MAX_SEQUENCE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    WorkerData,
    WorkerControl,
    Client,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    PullStdin,
    SequencedOutput,
    ReleaseAck,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Terminal {
    pub stdin: bool,
    pub stdout: bool,
    pub stderr: bool,
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Frame {
    Hello {
        protocol: u32,
        session_id: String,
        artifact_id: String,
        token: String,
        role: Role,
    },
    Ready {
        protocol: u32,
        session_id: String,
        artifact_id: String,
        capabilities: Vec<Capability>,
    },
    Classify {
        request_id: String,
        argv: Vec<String>,
    },
    Refused {
        request_id: String,
        reason: String,
    },
    Eligible {
        request_id: String,
    },
    Invoke {
        request_id: String,
    },
    Accepted {
        request_id: String,
    },
    Cold {
        request_id: String,
        reason: String,
    },
    StdinRead {
        request_id: String,
        max_bytes: u32,
    },
    Stdin {
        request_id: String,
        base64: String,
    },
    StdinEof {
        request_id: String,
    },
    Output {
        request_id: String,
        sequence: u64,
        stream: OutputStream,
        base64: String,
    },
    Complete {
        request_id: String,
        exit_code: u8,
    },
    ClientDone {
        request_id: String,
        #[serde(deserialize_with = "required_option")]
        exit_code: Option<u8>,
        #[serde(deserialize_with = "required_option")]
        signal: Option<u32>,
    },
    Release {
        request_id: String,
    },
    Released {
        request_id: String,
    },
    Fault {
        #[serde(deserialize_with = "required_option")]
        request_id: Option<String>,
        reason: String,
    },
    #[serde(deserialize_with = "empty_object")]
    Shutdown,
}

fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::deserialize(deserializer)
}

fn empty_object<'de, D>(deserializer: D) -> Result<(), D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}

    Empty::deserialize(deserializer).map(|_| ())
}

impl fmt::Debug for Frame {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Frame")
            .field("kind", &self.kind())
            .field("request_id", &self.request_id())
            .finish_non_exhaustive()
    }
}

impl Frame {
    pub fn request_id(&self) -> Option<&str> {
        match self {
            Self::Hello { .. } | Self::Ready { .. } | Self::Shutdown => None,
            Self::Fault { request_id, .. } => request_id.as_deref(),
            Self::Classify { request_id, .. }
            | Self::Refused { request_id, .. }
            | Self::Eligible { request_id }
            | Self::Invoke { request_id }
            | Self::Accepted { request_id }
            | Self::Cold { request_id, .. }
            | Self::StdinRead { request_id, .. }
            | Self::Stdin { request_id, .. }
            | Self::StdinEof { request_id }
            | Self::Output { request_id, .. }
            | Self::Complete { request_id, .. }
            | Self::ClientDone { request_id, .. }
            | Self::Release { request_id }
            | Self::Released { request_id } => Some(request_id),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::Ready { .. } => "ready",
            Self::Classify { .. } => "classify",
            Self::Refused { .. } => "refused",
            Self::Eligible { .. } => "eligible",
            Self::Invoke { .. } => "invoke",
            Self::Accepted { .. } => "accepted",
            Self::Cold { .. } => "cold",
            Self::StdinRead { .. } => "stdin_read",
            Self::Stdin { .. } => "stdin",
            Self::StdinEof { .. } => "stdin_eof",
            Self::Output { .. } => "output",
            Self::Complete { .. } => "complete",
            Self::ClientDone { .. } => "client_done",
            Self::Release { .. } => "release",
            Self::Released { .. } => "released",
            Self::Fault { .. } => "fault",
            Self::Shutdown => "shutdown",
        }
    }
}

pub fn read_message<R: Read, T: DeserializeOwned>(reader: &mut R) -> Result<Option<T>, String> {
    let mut prefix = [0; 4];
    let count = loop {
        match reader.read(&mut prefix) {
            Ok(0) => return Ok(None),
            Ok(count) => break count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("cannot read frame length ({:?})", error.kind())),
        }
    };
    reader
        .read_exact(&mut prefix[count..])
        .map_err(|error| format!("truncated frame length ({:?})", error.kind()))?;
    let length = u32::from_be_bytes(prefix) as usize;
    if !(1..=MAX_FRAME_BYTES).contains(&length) {
        return Err("frame length must be between 1 and 1048576 bytes".to_string());
    }
    let mut payload = vec![0; length];
    reader
        .read_exact(&mut payload)
        .map_err(|error| format!("truncated frame payload ({:?})", error.kind()))?;
    serde_json::from_slice(&payload).map(Some).map_err(|error| {
        format!(
            "invalid JSON frame at line {}, column {}",
            error.line(),
            error.column()
        )
    })
}

pub fn write_message<W: Write, T: Serialize>(writer: &mut W, message: &T) -> Result<(), String> {
    struct BoundedPayload(Vec<u8>);

    impl Write for BoundedPayload {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if bytes.len() > MAX_FRAME_BYTES - self.0.len() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "encoded frame exceeds 1048576 bytes",
                ));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    let mut payload = BoundedPayload(Vec::new());
    serde_json::to_writer(&mut payload, message)
        .map_err(|_| "cannot encode JSON frame within 1048576 bytes".to_string())?;
    let prefix = (payload.0.len() as u32).to_be_bytes();
    writer
        .write_all(&prefix)
        .and_then(|()| writer.write_all(&payload.0))
        .and_then(|()| writer.flush())
        .map_err(|error| format!("cannot write frame ({:?})", error.kind()))
}

pub fn read_frame(reader: &mut impl Read) -> Result<Option<Frame>, String> {
    let frame = read_message(reader)?;
    if let Some(frame) = &frame {
        validate_frame(frame)?;
    }
    Ok(frame)
}

pub fn write_frame(writer: &mut impl Write, frame: &Frame) -> Result<(), String> {
    validate_frame(frame)?;
    write_message(writer, frame)
}

pub fn validate_frame(frame: &Frame) -> Result<(), String> {
    if frame.request_id().is_some_and(str::is_empty) {
        return Err(format!("{} requires a nonempty request_id", frame.kind()));
    }
    match frame {
        Frame::Hello {
            protocol,
            session_id,
            artifact_id,
            token,
            ..
        } => {
            validate_identity(*protocol, session_id, artifact_id)?;
            if token.is_empty() {
                return Err("hello requires a nonempty token".to_string());
            }
        }
        Frame::Ready {
            protocol,
            session_id,
            artifact_id,
            capabilities,
        } => {
            validate_identity(*protocol, session_id, artifact_id)?;
            if capabilities.len() != 3
                || !capabilities.contains(&Capability::PullStdin)
                || !capabilities.contains(&Capability::SequencedOutput)
                || !capabilities.contains(&Capability::ReleaseAck)
            {
                return Err("ready requires exactly the three protocol-1 capabilities".to_string());
            }
        }
        Frame::StdinRead { max_bytes, .. } if !(1..=MAX_CHUNK_BYTES as u32).contains(max_bytes) => {
            return Err("stdin_read max_bytes must be between 1 and 65536".to_string());
        }
        Frame::Output {
            sequence, base64, ..
        } => {
            if *sequence > MAX_SEQUENCE {
                return Err("output sequence exceeds 9007199254740991".to_string());
            }
            decode_chunk(base64)?;
        }
        Frame::Stdin { base64, .. } => {
            decode_chunk(base64)?;
        }
        Frame::ClientDone {
            exit_code, signal, ..
        } => {
            if exit_code.is_some() == signal.is_some() || *signal == Some(0) {
                return Err(
                    "client_done requires exactly one exit_code or positive signal".to_string(),
                );
            }
        }
        Frame::Refused { reason, .. }
        | Frame::Cold { reason, .. }
        | Frame::Fault { reason, .. }
            if reason.is_empty() =>
        {
            return Err(format!("{} requires a nonempty reason", frame.kind()));
        }
        _ => {}
    }
    Ok(())
}

fn validate_identity(protocol: u32, session_id: &str, artifact_id: &str) -> Result<(), String> {
    if protocol != 1 {
        return Err("unsupported session protocol".to_string());
    }
    if session_id.is_empty() || artifact_id.is_empty() {
        return Err("session and artifact identities must be nonempty".to_string());
    }
    Ok(())
}

pub fn decode_chunk(encoded: &str) -> Result<Vec<u8>, String> {
    if encoded.is_empty() || encoded.len() > MAX_CHUNK_BYTES.div_ceil(3) * 4 {
        return Err("binary chunks must contain between 1 and 65536 bytes".to_string());
    }
    let decoded = STANDARD
        .decode(encoded)
        .map_err(|_| "binary chunk is not canonical padded Base64".to_string())?;
    if decoded.is_empty() || decoded.len() > MAX_CHUNK_BYTES {
        return Err("binary chunks must contain between 1 and 65536 bytes".to_string());
    }
    if STANDARD.encode(&decoded) != encoded {
        return Err("binary chunk is not canonical padded Base64".to_string());
    }
    Ok(decoded)
}

pub fn encode_chunk(bytes: &[u8]) -> Result<String, String> {
    if !(1..=MAX_CHUNK_BYTES).contains(&bytes.len()) {
        return Err("binary chunks must contain between 1 and 65536 bytes".to_string());
    }
    Ok(STANDARD.encode(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn framed(json: &str) -> Vec<u8> {
        let mut bytes = u32::try_from(json.len()).unwrap().to_be_bytes().to_vec();
        bytes.extend_from_slice(json.as_bytes());
        bytes
    }

    fn parse(json: &str) -> Result<Option<Frame>, String> {
        read_frame(&mut Cursor::new(framed(json)))
    }

    #[test]
    fn frame_roundtrip_preserves_typed_binary_output() {
        let frame = Frame::Output {
            request_id: "call-1".to_string(),
            sequence: MAX_SEQUENCE,
            stream: OutputStream::Stderr,
            base64: "AP9cCg==".to_string(),
        };
        let mut bytes = Vec::new();
        write_frame(&mut bytes, &frame).unwrap();
        assert_eq!(
            u32::from_be_bytes(bytes[..4].try_into().unwrap()) as usize,
            bytes.len() - 4
        );
        assert!(read_frame(&mut Cursor::new(bytes)).unwrap() == Some(frame));
    }

    #[test]
    fn shared_envelope_preserves_separate_internal_message_schemas() {
        #[derive(Deserialize, PartialEq, Serialize)]
        #[serde(deny_unknown_fields)]
        struct Internal {
            accepted: bool,
        }
        let mut bytes = Vec::new();
        write_message(&mut bytes, &Internal { accepted: true }).unwrap();
        let result: Option<Internal> = read_message(&mut Cursor::new(bytes)).unwrap();
        assert!(result == Some(Internal { accepted: true }));
    }

    #[test]
    fn fragmented_and_interrupted_reads_preserve_frame_boundaries() {
        struct Fragmented {
            bytes: Cursor<Vec<u8>>,
            reads: usize,
        }

        impl Read for Fragmented {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.reads += 1;
                if self.reads % 3 == 1 {
                    return Err(std::io::ErrorKind::Interrupted.into());
                }
                let limit = buffer.len().min(1);
                self.bytes.read(&mut buffer[..limit])
            }
        }

        let mut bytes = framed(r#"{"kind":"eligible","request_id":"first"}"#);
        bytes.extend(framed(r#"{"kind":"released","request_id":"second"}"#));
        let mut reader = Fragmented {
            bytes: Cursor::new(bytes),
            reads: 0,
        };
        let first = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(first.kind(), "eligible");
        assert_eq!(first.request_id(), Some("first"));
        let second = read_frame(&mut reader).unwrap().unwrap();
        assert_eq!(second.kind(), "released");
        assert_eq!(second.request_id(), Some("second"));
        assert!(read_frame(&mut reader).unwrap().is_none());
    }

    #[test]
    fn frame_debug_and_parse_errors_exclude_authentication_tokens() {
        let frame = Frame::Hello {
            protocol: 1,
            session_id: "session".to_string(),
            artifact_id: "artifact".to_string(),
            token: "private-token".to_string(),
            role: Role::Client,
        };
        assert!(!format!("{frame:?}").contains("private-token"));
        let invalid = r#"{"kind":"hello","protocol":"private-token","session_id":"session","artifact_id":"artifact","token":"private-token","role":"client"}"#;
        let error = parse(invalid).unwrap_err();
        assert!(!error.contains("private-token"));
    }

    #[test]
    fn clean_eof_is_distinct_from_truncated_prefix_and_body() {
        assert!(read_frame(&mut Cursor::new([])).unwrap().is_none());
        for bytes in [vec![0], vec![0, 0, 0], vec![0, 0, 0, 2, b'{']] {
            assert!(read_frame(&mut Cursor::new(bytes)).is_err());
        }
        assert!(matches!(
            parse(r#"{"kind":"shutdown"}"#),
            Ok(Some(Frame::Shutdown))
        ));
    }

    #[test]
    fn frame_lengths_are_bounded_before_allocation() {
        for length in [0, MAX_FRAME_BYTES as u32 + 1, u32::MAX] {
            assert!(read_frame(&mut Cursor::new(length.to_be_bytes())).is_err());
        }
        let frame = Frame::Classify {
            request_id: "id".to_string(),
            argv: vec!["x".repeat(MAX_FRAME_BYTES)],
        };
        assert!(write_frame(&mut Vec::new(), &frame).is_err());
    }

    #[test]
    fn frame_parser_rejects_unknown_fields_types_and_non_json_bytes() {
        for json in [
            r#"{"kind":"unknown"}"#,
            r#"{"kind":"shutdown","extra":1}"#,
            r#"{"kind":"eligible","request_id":"id","extra":1}"#,
            r#"{"kind":"eligible","request_id":1}"#,
            r#"{"kind":"eligible"}"#,
            r#"{"kind":"complete","request_id":"id","exit_code":256}"#,
            r#"{"kind":"complete","request_id":"id","exit_code":-1}"#,
            r#"{"kind":"complete","request_id":"id","exit_code":1.5}"#,
            r#"{"kind":"output","request_id":"id","sequence":-1,"stream":"stdout","base64":"eA=="}"#,
            r#"{"kind":"client_done","request_id":"id","exit_code":0}"#,
            r#"{"kind":"fault","reason":"broken"}"#,
        ] {
            assert!(parse(json).is_err(), "unexpected valid frame: {json}");
        }
        assert!(read_frame(&mut Cursor::new(vec![0, 0, 0, 1, 255])).is_err());
    }

    #[test]
    fn frame_parser_validates_request_ids_sequence_and_statuses() {
        for json in [
            r#"{"kind":"eligible","request_id":""}"#,
            r#"{"kind":"output","request_id":"id","sequence":9007199254740992,"stream":"stdout","base64":"eA=="}"#,
            r#"{"kind":"stdin_read","request_id":"id","max_bytes":0}"#,
            r#"{"kind":"stdin_read","request_id":"id","max_bytes":65537}"#,
            r#"{"kind":"client_done","request_id":"id","exit_code":null,"signal":null}"#,
            r#"{"kind":"client_done","request_id":"id","exit_code":0,"signal":9}"#,
            r#"{"kind":"client_done","request_id":"id","exit_code":null,"signal":0}"#,
            r#"{"kind":"fault","request_id":"","reason":"broken"}"#,
        ] {
            assert!(parse(json).is_err(), "unexpected valid frame: {json}");
        }
        assert!(
            parse(r#"{"kind":"client_done","request_id":"id","exit_code":255,"signal":null}"#)
                .is_ok()
        );
        assert!(
            parse(r#"{"kind":"client_done","request_id":"id","exit_code":null,"signal":9}"#)
                .is_ok()
        );
        assert!(parse(r#"{"kind":"fault","request_id":null,"reason":"worker_exit"}"#).is_ok());
    }

    #[test]
    fn terminal_types_are_strict() {
        let json = r#"{"stdin":false,"stdout":true,"stderr":false}"#;
        assert!(serde_json::from_str::<Terminal>(json).is_ok());
        for invalid in [
            json.replace("false", "0"),
            json.replace(r#""stderr":false"#, r#""stderr":false,"extra":true"#),
        ] {
            assert!(serde_json::from_str::<Terminal>(&invalid).is_err());
        }
    }

    #[test]
    fn identity_and_mandatory_capabilities_are_validated() {
        let ready = r#"{"kind":"ready","protocol":1,"session_id":"session","artifact_id":"artifact","capabilities":["pull_stdin","sequenced_output","release_ack"]}"#;
        assert!(parse(ready).is_ok());
        for invalid in [
            ready.replace("\"protocol\":1", "\"protocol\":2"),
            ready.replace("session\"", "\""),
            ready.replace(",\"release_ack\"", ""),
            ready.replace("release_ack", "pull_stdin"),
            ready.replace("release_ack", "unknown"),
        ] {
            assert!(parse(&invalid).is_err());
        }
    }

    #[test]
    fn chunks_require_nonempty_canonical_padded_base64() {
        assert_eq!(decode_chunk("AP9cCg==").unwrap(), [0, 255, 92, 10]);
        assert_eq!(encode_chunk(&[0, 255, 92, 10]).unwrap(), "AP9cCg==");
        for invalid in ["", "Zg", "Zg=", "Zh==", "Zg===", "Zg==\n", "_w==", "!!!!"] {
            assert!(decode_chunk(invalid).is_err(), "accepted {invalid:?}");
        }
        assert!(encode_chunk(&[]).is_err());
        assert!(encode_chunk(&vec![1; MAX_CHUNK_BYTES + 1]).is_err());
        let maximum = vec![0; MAX_CHUNK_BYTES];
        assert_eq!(
            decode_chunk(&encode_chunk(&maximum).unwrap()).unwrap(),
            maximum
        );
        let too_large = "AAAA".repeat((MAX_CHUNK_BYTES + 3) / 3);
        assert!(decode_chunk(&too_large).is_err());
    }

    #[test]
    fn framing_does_not_limit_total_stream_size() {
        let chunk = encode_chunk(&vec![42; MAX_CHUNK_BYTES]).unwrap();
        let mut bytes = Vec::new();
        for sequence in 0..20 {
            write_frame(
                &mut bytes,
                &Frame::Output {
                    request_id: "id".to_string(),
                    sequence,
                    stream: OutputStream::Stdout,
                    base64: chunk.clone(),
                },
            )
            .unwrap();
        }
        assert!(bytes.len() > MAX_FRAME_BYTES);
        let mut cursor = Cursor::new(bytes);
        let mut decoded = 0;
        while let Some(Frame::Output { base64, .. }) = read_frame(&mut cursor).unwrap() {
            decoded += decode_chunk(&base64).unwrap().len();
        }
        assert_eq!(decoded, 20 * MAX_CHUNK_BYTES);
    }
}

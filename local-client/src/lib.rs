//! Loopback HTTP adapter for Dendrite's typed Tellus presentation boundary.
#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tellus::{
    EntryV1, GroundingStaleReasonV1, GroundingVerificationPort, GroundingVerificationRequestV1,
    GroundingVerificationV1, ObservationV1, TellusPort, VerifiedGroundingSourceV1,
    VerifiedGroundingSourcesV1,
};

const MAX_RESPONSE_BYTES: u64 = 256 * 1024;

#[derive(Debug)]
pub enum ClientError {
    NotSent(io::Error),
    OutcomeUnknown(io::Error),
    InvalidResponse,
    Refused(u16),
}

pub struct LocalPort {
    endpoint: SocketAddr,
    bearer_token: Option<String>,
    timeout: Duration,
}

impl LocalPort {
    pub fn new(
        endpoint: SocketAddr,
        bearer_token: Option<String>,
        timeout: Duration,
    ) -> Result<Self, ClientError> {
        if !endpoint.ip().is_loopback() || timeout.is_zero() {
            return Err(ClientError::InvalidResponse);
        }
        if bearer_token.as_ref().is_some_and(|token| {
            token.is_empty() || token.len() > 8192 || token.contains(['\r', '\n'])
        }) {
            return Err(ClientError::InvalidResponse);
        }
        Ok(Self {
            endpoint,
            bearer_token,
            timeout,
        })
    }

    fn post(&self, path: &str, body: &[u8]) -> Result<Vec<u8>, ClientError> {
        let mut stream = TcpStream::connect_timeout(&self.endpoint, self.timeout)
            .map_err(ClientError::NotSent)?;
        stream
            .set_read_timeout(Some(self.timeout))
            .and_then(|()| stream.set_write_timeout(Some(self.timeout)))
            .map_err(ClientError::NotSent)?;
        let authorization = self
            .bearer_token
            .as_ref()
            .map_or_else(String::new, |token| {
                format!("Authorization: Bearer {token}\r\n")
            });
        let request = format!(
            "POST {path} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n",
            self.endpoint,
            body.len(),
            authorization,
        );
        stream
            .write_all(request.as_bytes())
            .and_then(|()| stream.write_all(body))
            .map_err(ClientError::OutcomeUnknown)?;
        stream.flush().map_err(ClientError::OutcomeUnknown)?;
        let mut bytes = Vec::new();
        stream
            .take(MAX_RESPONSE_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(ClientError::OutcomeUnknown)?;
        if bytes.len() as u64 > MAX_RESPONSE_BYTES {
            return Err(ClientError::InvalidResponse);
        }
        let separator = bytes
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .ok_or(ClientError::InvalidResponse)?;
        let headers =
            std::str::from_utf8(&bytes[..separator]).map_err(|_| ClientError::InvalidResponse)?;
        let status = headers
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u16>().ok())
            .ok_or(ClientError::InvalidResponse)?;
        if status != 200 {
            return Err(ClientError::Refused(status));
        }
        Ok(bytes[separator + 4..].to_vec())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseV1 {
    version: u8,
    capability: String,
    status: String,
    complete: bool,
    entries: Vec<ResponseEntryV1>,
    truncated: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseEntryV1 {
    load_id: String,
    state: String,
    freshness: String,
    sha256: Option<String>,
}

#[derive(Serialize)]
struct GroundingRequestWireV1 {
    version: u8,
    method_sha256: String,
    fold_sha256: String,
    orthogonality_sha256: String,
    selections: Vec<GroundingSelectionWireV1>,
}

#[derive(Serialize)]
struct GroundingSelectionWireV1 {
    member: String,
    authority_sha256: String,
    surface_sha256: String,
    depth: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundingResponseV1 {
    version: u8,
    capability: String,
    status: String,
    complete: bool,
    #[serde(default)]
    reason: Option<String>,
    sources: std::collections::BTreeMap<String, GroundingSourceResponseV1>,
    selections: Vec<GroundingSelectionResponseV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundingSourceResponseV1 {
    load_id: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GroundingSelectionResponseV1 {
    member: String,
    verified: bool,
}

impl TellusPort for LocalPort {
    type Error = ClientError;

    fn observe(&mut self, limit: usize) -> Result<ObservationV1, Self::Error> {
        if !(1..=64).contains(&limit) {
            return Err(ClientError::InvalidResponse);
        }
        let body = format!("{{\"version\":1,\"limit\":{limit}}}");
        let response: ResponseV1 =
            serde_json::from_slice(&self.post("/tellus/v1/observe", body.as_bytes())?)
                .map_err(|_| ClientError::InvalidResponse)?;
        if response.version != 1
            || response.capability != "tellus.observe.v1"
            || response.entries.len() > limit
        {
            return Err(ClientError::InvalidResponse);
        }
        let entries = response
            .entries
            .into_iter()
            .map(|entry| {
                if entry.load_id.is_empty()
                    || entry.load_id.len() > 256
                    || entry.state.len() > 32
                    || entry.freshness.len() > 32
                    || entry.sha256.as_ref().is_some_and(|digest| {
                        digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                    })
                {
                    return Err(ClientError::InvalidResponse);
                }
                Ok(EntryV1 {
                    load_id: entry.load_id,
                    state: entry.state,
                    freshness: entry.freshness,
                    sha256: entry.sha256,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ObservationV1 {
            complete: response.complete,
            status: response.status,
            entries,
            truncated: response.truncated,
        })
    }
}

impl GroundingVerificationPort for LocalPort {
    type Error = ClientError;

    fn verify_grounding(
        &mut self,
        request: &GroundingVerificationRequestV1,
    ) -> Result<GroundingVerificationV1, Self::Error> {
        if request.selections.is_empty() || request.selections.len() > 16 {
            return Err(ClientError::InvalidResponse);
        }
        let members = request
            .selections
            .iter()
            .map(|selection| selection.member)
            .collect::<BTreeSet<_>>();
        if members.len() != request.selections.len() {
            return Err(ClientError::InvalidResponse);
        }
        let wire = GroundingRequestWireV1 {
            version: 1,
            method_sha256: lower_hex(&request.method_sha256),
            fold_sha256: lower_hex(&request.fold_sha256),
            orthogonality_sha256: lower_hex(&request.orthogonality_sha256),
            selections: request
                .selections
                .iter()
                .map(|selection| GroundingSelectionWireV1 {
                    member: lower_hex(&selection.member),
                    authority_sha256: lower_hex(&selection.authority_sha256),
                    surface_sha256: lower_hex(&selection.surface_sha256),
                    depth: selection.depth.as_str(),
                })
                .collect(),
        };
        let body = serde_json::to_vec(&wire).map_err(|_| ClientError::InvalidResponse)?;
        let response: GroundingResponseV1 =
            serde_json::from_slice(&self.post("/tellus/v1/verify-grounding", &body)?)
                .map_err(|_| ClientError::InvalidResponse)?;
        if response.version != 1 || response.capability != "tellus.verify-grounding.v1" {
            return Err(ClientError::InvalidResponse);
        }
        match response.status.as_str() {
            "current" if response.complete && response.reason.is_none() => {
                current_grounding(response, request)
            }
            "stale"
                if !response.complete
                    && response.sources.is_empty()
                    && response.selections.is_empty() =>
            {
                Ok(GroundingVerificationV1::Stale(stale_reason(
                    response.reason.as_deref(),
                )?))
            }
            _ => Err(ClientError::InvalidResponse),
        }
    }
}

fn current_grounding(
    mut response: GroundingResponseV1,
    request: &GroundingVerificationRequestV1,
) -> Result<GroundingVerificationV1, ClientError> {
    let method = verified_source(response.sources.remove("method"), request.method_sha256)?;
    let fold = verified_source(response.sources.remove("fold"), request.fold_sha256)?;
    let orthogonality = verified_source(
        response.sources.remove("orthogonality"),
        request.orthogonality_sha256,
    )?;
    if !response.sources.is_empty() || response.selections.len() != request.selections.len() {
        return Err(ClientError::InvalidResponse);
    }
    let members = response
        .selections
        .into_iter()
        .zip(&request.selections)
        .map(|(actual, expected)| {
            let member = parse_hex::<16>(&actual.member).ok_or(ClientError::InvalidResponse)?;
            if !actual.verified || member != expected.member {
                return Err(ClientError::InvalidResponse);
            }
            Ok(member)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GroundingVerificationV1::Current {
        sources: VerifiedGroundingSourcesV1 {
            method,
            fold,
            orthogonality,
        },
        members,
    })
}

fn verified_source(
    source: Option<GroundingSourceResponseV1>,
    expected: [u8; 32],
) -> Result<VerifiedGroundingSourceV1, ClientError> {
    let source = source.ok_or(ClientError::InvalidResponse)?;
    let sha256 = parse_hex::<32>(&source.sha256).ok_or(ClientError::InvalidResponse)?;
    if source.load_id.is_empty()
        || source.load_id.len() > 256
        || source.load_id.chars().any(char::is_control)
        || sha256 != expected
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(VerifiedGroundingSourceV1 {
        load_id: source.load_id,
        sha256,
    })
}

fn stale_reason(value: Option<&str>) -> Result<GroundingStaleReasonV1, ClientError> {
    match value {
        Some("source_missing") => Ok(GroundingStaleReasonV1::SourceMissing),
        Some("source_changed") => Ok(GroundingStaleReasonV1::SourceChanged),
        Some("grounding_missing") => Ok(GroundingStaleReasonV1::GroundingMissing),
        Some("grounding_mismatch") => Ok(GroundingStaleReasonV1::GroundingMismatch),
        _ => Err(ClientError::InvalidResponse),
    }
}

fn lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        value.push(char::from(HEX[usize::from(byte >> 4)]));
        value.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    value
}

fn parse_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N.checked_mul(2)?
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    let mut bytes = [0; N];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Some(bytes)
}

const fn hex_nibble(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tellus::{GroundingDepthV1, GroundingSelectionV1};

    fn request() -> GroundingVerificationRequestV1 {
        GroundingVerificationRequestV1 {
            method_sha256: [1; 32],
            fold_sha256: [2; 32],
            orthogonality_sha256: [3; 32],
            selections: vec![GroundingSelectionV1 {
                member: [4; 16],
                authority_sha256: [5; 32],
                surface_sha256: [6; 32],
                depth: GroundingDepthV1::Activated,
            }],
        }
    }

    #[test]
    fn exact_current_response_preserves_sources_and_members() {
        let request = request();
        let response = GroundingResponseV1 {
            version: 1,
            capability: "tellus.verify-grounding.v1".into(),
            status: "current".into(),
            complete: true,
            reason: None,
            sources: [
                (
                    "method".into(),
                    GroundingSourceResponseV1 {
                        load_id: "method".into(),
                        sha256: lower_hex(&request.method_sha256),
                    },
                ),
                (
                    "fold".into(),
                    GroundingSourceResponseV1 {
                        load_id: "fold".into(),
                        sha256: lower_hex(&request.fold_sha256),
                    },
                ),
                (
                    "orthogonality".into(),
                    GroundingSourceResponseV1 {
                        load_id: "lattice".into(),
                        sha256: lower_hex(&request.orthogonality_sha256),
                    },
                ),
            ]
            .into(),
            selections: vec![GroundingSelectionResponseV1 {
                member: lower_hex(&request.selections[0].member),
                verified: true,
            }],
        };
        assert_eq!(
            current_grounding(response, &request).unwrap(),
            GroundingVerificationV1::Current {
                sources: VerifiedGroundingSourcesV1 {
                    method: VerifiedGroundingSourceV1 {
                        load_id: "method".into(),
                        sha256: [1; 32],
                    },
                    fold: VerifiedGroundingSourceV1 {
                        load_id: "fold".into(),
                        sha256: [2; 32],
                    },
                    orthogonality: VerifiedGroundingSourceV1 {
                        load_id: "lattice".into(),
                        sha256: [3; 32],
                    },
                },
                members: vec![[4; 16]],
            }
        );
    }

    #[test]
    fn response_decoding_rejects_unbound_or_noncanonical_values() {
        assert_eq!(parse_hex::<2>("00af"), Some([0, 175]));
        assert_eq!(parse_hex::<2>("00AF"), None);
        assert!(
            verified_source(
                Some(GroundingSourceResponseV1 {
                    load_id: "bad\nload".into(),
                    sha256: lower_hex(&[1; 32]),
                }),
                [1; 32],
            )
            .is_err()
        );
        assert_eq!(
            stale_reason(Some("grounding_mismatch")).unwrap(),
            GroundingStaleReasonV1::GroundingMismatch
        );
        assert!(stale_reason(Some("invented")).is_err());
    }
}

//! Loopback HTTP adapter for Dendrite's typed Grounding presentation boundary.
#![forbid(unsafe_code)]

use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use serde::Deserialize;
use tellus::{EntryV1, GroundingPort, ObservationV1};

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

impl GroundingPort for LocalPort {
    type Error = ClientError;

    fn observe(&mut self, limit: usize) -> Result<ObservationV1, Self::Error> {
        if !(1..=64).contains(&limit) {
            return Err(ClientError::InvalidResponse);
        }
        let body = format!("{{\"version\":1,\"limit\":{limit}}}");
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
            "POST /grounding/v1/observe HTTP/1.1\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{}\r\n{}",
            self.endpoint,
            body.len(),
            authorization,
            body
        );
        stream
            .write_all(request.as_bytes())
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
        let response: ResponseV1 = serde_json::from_slice(&bytes[separator + 4..])
            .map_err(|_| ClientError::InvalidResponse)?;
        if response.version != 1
            || response.capability != "grounding.observe.v1"
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

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
    GroundingVerificationV1, ObservationV1, SME_ADVISORY_BOUNDARY, SME_ENSEMBLE_QUESTION,
    SmeEntryKindV1, SmeEntryV1, SmeOrthogonalityPairV1, SmeOrthogonalityV1, SmePlanPort,
    SmePlanRequestV1, SmePlanSourceV1, SmePlanStatusV1, SmePlanV1, SmeRoleV1, TellusPort,
    VerifiedGroundingSourceV1, VerifiedGroundingSourcesV1,
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

#[derive(Serialize)]
struct SmePlanRequestWireV1<'a> {
    version: u8,
    fold_load_id: &'a str,
    lattice_load_id: &'a str,
    tags: &'a [String],
    depth: &'static str,
    max_roles: u8,
    max_surface_bytes: u32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmePlanResponseV1 {
    version: u8,
    capability: String,
    status: String,
    complete: bool,
    next_action: String,
    fold: SmePlanSourceResponseV1,
    lattice: SmePlanSourceResponseV1,
    tags: Vec<String>,
    roles: Vec<SmeRoleResponseV1>,
    orthogonality: SmeOrthogonalityResponseV1,
    ensemble_question: String,
    boundary: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmePlanSourceResponseV1 {
    load_id: String,
    sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmeRoleResponseV1 {
    role: String,
    #[serde(rename = "ref")]
    reference: String,
    authority_sha256: String,
    surface_sha256: String,
    depth: String,
    surface_version: u8,
    entries: Vec<SmeEntryResponseV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmeEntryResponseV1 {
    #[serde(default)]
    id: Option<String>,
    kind: String,
    text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmeOrthogonalityResponseV1 {
    all_orthogonal: bool,
    sufficient_roles: bool,
    pairs: Vec<SmeOrthogonalityPairResponseV1>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SmeOrthogonalityPairResponseV1 {
    role1: String,
    role2: String,
    orthogonal: bool,
    facet1: String,
    facet2: String,
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

impl SmePlanPort for LocalPort {
    type Error = ClientError;

    fn plan_smes(&mut self, request: &SmePlanRequestV1) -> Result<SmePlanV1, Self::Error> {
        validate_sme_request(request)?;
        let wire = SmePlanRequestWireV1 {
            version: 1,
            fold_load_id: &request.fold_load_id,
            lattice_load_id: &request.lattice_load_id,
            tags: &request.tags,
            depth: request.depth.as_str(),
            max_roles: request.max_roles,
            max_surface_bytes: request.max_surface_bytes,
        };
        let body = serde_json::to_vec(&wire).map_err(|_| ClientError::InvalidResponse)?;
        let response: SmePlanResponseV1 =
            serde_json::from_slice(&self.post("/tellus/v1/sme-plan", &body)?)
                .map_err(|_| ClientError::InvalidResponse)?;
        decode_sme_plan(response, request)
    }
}

fn validate_sme_request(request: &SmePlanRequestV1) -> Result<(), ClientError> {
    if !valid_text(&request.fold_load_id, 256)
        || !valid_text(&request.lattice_load_id, 256)
        || request.tags.is_empty()
        || request.tags.len() > 16
        || !(1..=8).contains(&request.max_roles)
        || !(1..=131_072).contains(&request.max_surface_bytes)
        || request.tags.iter().any(|tag| !valid_text(tag, 128))
        || request.tags.iter().collect::<BTreeSet<_>>().len() != request.tags.len()
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(())
}

fn decode_sme_plan(
    response: SmePlanResponseV1,
    request: &SmePlanRequestV1,
) -> Result<SmePlanV1, ClientError> {
    if response.version != 1
        || response.capability != "tellus.sme-plan.v1"
        || response.ensemble_question != SME_ENSEMBLE_QUESTION
        || response.boundary != SME_ADVISORY_BOUNDARY
        || response.tags != request.tags
        || response.roles.len() > usize::from(request.max_roles)
        || !valid_text(&response.next_action, 1_024)
    {
        return Err(ClientError::InvalidResponse);
    }
    let status = match response.status.as_str() {
        "current" if response.complete => SmePlanStatusV1::Current,
        "review_required" if !response.complete => SmePlanStatusV1::ReviewRequired,
        "unmatched" if !response.complete => SmePlanStatusV1::Unmatched,
        _ => return Err(ClientError::InvalidResponse),
    };
    let fold = decode_sme_source(response.fold, &request.fold_load_id)?;
    let lattice = decode_sme_source(response.lattice, &request.lattice_load_id)?;
    let roles = response
        .roles
        .into_iter()
        .map(|role| decode_sme_role(role, request.depth))
        .collect::<Result<Vec<_>, _>>()?;
    let orthogonality = decode_orthogonality(response.orthogonality, roles.len())?;
    if matches!(status, SmePlanStatusV1::Current)
        != (orthogonality.sufficient_roles && orthogonality.all_orthogonal)
        || matches!(status, SmePlanStatusV1::Unmatched) != roles.is_empty()
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(SmePlanV1 {
        status,
        complete: response.complete,
        next_action: response.next_action,
        fold,
        lattice,
        tags: response.tags,
        roles,
        orthogonality,
    })
}

fn decode_sme_source(
    source: SmePlanSourceResponseV1,
    expected_load_id: &str,
) -> Result<SmePlanSourceV1, ClientError> {
    let sha256 = parse_hex::<32>(&source.sha256).ok_or(ClientError::InvalidResponse)?;
    if source.load_id != expected_load_id {
        return Err(ClientError::InvalidResponse);
    }
    Ok(SmePlanSourceV1 {
        load_id: source.load_id,
        sha256,
    })
}

fn decode_sme_role(
    role: SmeRoleResponseV1,
    expected_depth: tellus::GroundingDepthV1,
) -> Result<SmeRoleV1, ClientError> {
    if !valid_text(&role.role, 256)
        || !valid_text(&role.reference, 256)
        || role.depth != expected_depth.as_str()
        || !(1..=2).contains(&role.surface_version)
        || role.entries.is_empty()
        || role.entries.len() > 64
    {
        return Err(ClientError::InvalidResponse);
    }
    let entries = role
        .entries
        .into_iter()
        .map(|entry| decode_sme_entry(entry, role.surface_version))
        .collect::<Result<Vec<_>, _>>()?;
    let ids = entries
        .iter()
        .filter_map(|entry| entry.id.as_ref())
        .collect::<BTreeSet<_>>();
    if (role.surface_version == 1 && !ids.is_empty())
        || (role.surface_version == 2 && ids.len() != entries.len())
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(SmeRoleV1 {
        role: role.role,
        reference: role.reference,
        authority_sha256: parse_hex(&role.authority_sha256).ok_or(ClientError::InvalidResponse)?,
        surface_sha256: parse_hex(&role.surface_sha256).ok_or(ClientError::InvalidResponse)?,
        depth: expected_depth,
        surface_version: role.surface_version,
        entries,
    })
}

fn decode_sme_entry(
    entry: SmeEntryResponseV1,
    surface_version: u8,
) -> Result<SmeEntryV1, ClientError> {
    let kind = match entry.kind.as_str() {
        "guidance" => SmeEntryKindV1::Guidance,
        "question" => SmeEntryKindV1::Question,
        "hazard" => SmeEntryKindV1::Hazard,
        "probe" => SmeEntryKindV1::Probe,
        "reference" => SmeEntryKindV1::Reference,
        _ => return Err(ClientError::InvalidResponse),
    };
    if entry.text.trim().is_empty()
        || entry.text.len() > 8_192
        || (surface_version == 1 && entry.id.is_some())
        || (surface_version == 2 && !entry.id.as_ref().is_some_and(|id| valid_entry_id(id)))
    {
        return Err(ClientError::InvalidResponse);
    }
    Ok(SmeEntryV1 {
        id: entry.id,
        kind,
        text: entry.text,
    })
}

fn decode_orthogonality(
    value: SmeOrthogonalityResponseV1,
    role_count: usize,
) -> Result<SmeOrthogonalityV1, ClientError> {
    if value.sufficient_roles != (role_count >= 2)
        || (role_count < 2 && (!value.pairs.is_empty() || value.all_orthogonal))
    {
        return Err(ClientError::InvalidResponse);
    }
    let pairs = value
        .pairs
        .into_iter()
        .map(|pair| {
            if [
                pair.role1.as_str(),
                pair.role2.as_str(),
                pair.facet1.as_str(),
                pair.facet2.as_str(),
            ]
            .into_iter()
            .any(|value| !valid_text(value, 256))
            {
                return Err(ClientError::InvalidResponse);
            }
            Ok(SmeOrthogonalityPairV1 {
                role1: pair.role1,
                role2: pair.role2,
                orthogonal: pair.orthogonal,
                facet1: pair.facet1,
                facet2: pair.facet2,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SmeOrthogonalityV1 {
        all_orthogonal: value.all_orthogonal,
        sufficient_roles: value.sufficient_roles,
        pairs,
    })
}

fn valid_text(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty() && value.len() <= maximum_bytes && !value.chars().any(char::is_control)
}

fn valid_entry_id(value: &str) -> bool {
    let mut segments = value.split('-');
    let Some(first) = segments.next() else {
        return false;
    };
    !first.is_empty()
        && first.as_bytes()[0].is_ascii_lowercase()
        && first
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && segments.all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
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

    fn sme_request() -> SmePlanRequestV1 {
        SmePlanRequestV1 {
            fold_load_id: "fold".into(),
            lattice_load_id: "lattice".into(),
            tags: vec!["data-handling".into()],
            depth: GroundingDepthV1::Ambient,
            max_roles: 4,
            max_surface_bytes: 65_536,
        }
    }

    fn sme_response() -> SmePlanResponseV1 {
        SmePlanResponseV1 {
            version: 1,
            capability: "tellus.sme-plan.v1".into(),
            status: "review_required".into(),
            complete: false,
            next_action: "add independent coverage".into(),
            fold: SmePlanSourceResponseV1 {
                load_id: "fold".into(),
                sha256: lower_hex(&[1; 32]),
            },
            lattice: SmePlanSourceResponseV1 {
                load_id: "lattice".into(),
                sha256: lower_hex(&[2; 32]),
            },
            tags: vec!["data-handling".into()],
            roles: vec![SmeRoleResponseV1 {
                role: "privacy".into(),
                reference: "rune:privacy".into(),
                authority_sha256: lower_hex(&[3; 32]),
                surface_sha256: lower_hex(&[4; 32]),
                depth: "ambient".into(),
                surface_version: 2,
                entries: vec![SmeEntryResponseV1 {
                    id: Some("purpose-boundary".into()),
                    kind: "guidance".into(),
                    text: "retain the declared purpose".into(),
                }],
            }],
            orthogonality: SmeOrthogonalityResponseV1 {
                all_orthogonal: false,
                sufficient_roles: false,
                pairs: vec![],
            },
            ensemble_question: SME_ENSEMBLE_QUESTION.into(),
            boundary: SME_ADVISORY_BOUNDARY.into(),
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

    #[test]
    fn sme_plan_preserves_bounded_provenance_and_requires_review() {
        let plan = decode_sme_plan(sme_response(), &sme_request()).unwrap();
        assert_eq!(plan.status, SmePlanStatusV1::ReviewRequired);
        assert!(!plan.complete);
        assert_eq!(plan.fold.sha256, [1; 32]);
        assert_eq!(plan.lattice.sha256, [2; 32]);
        assert_eq!(plan.roles[0].authority_sha256, [3; 32]);
        assert_eq!(plan.roles[0].surface_sha256, [4; 32]);
        assert_eq!(
            plan.roles[0].entries[0].id.as_deref(),
            Some("purpose-boundary")
        );
    }

    #[test]
    fn sme_plan_rejects_advisory_leaks_and_invented_surface_grammar() {
        let request = sme_request();
        let mut wrong_question = sme_response();
        wrong_question.ensemble_question = "looks good?".into();
        assert!(decode_sme_plan(wrong_question, &request).is_err());

        let mut invented_kind = sme_response();
        invented_kind.roles[0].entries[0].kind = "command".into();
        assert!(decode_sme_plan(invented_kind, &request).is_err());

        let mut false_completion = sme_response();
        false_completion.status = "current".into();
        false_completion.complete = true;
        assert!(decode_sme_plan(false_completion, &request).is_err());
    }
}

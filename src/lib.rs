//! Composable Tellus presentation over a typed, non-MCP client boundary.
#![forbid(unsafe_code)]

use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    text::Line,
    widgets::{Block, Borders, Paragraph},
};

pub const OBSERVATION_LIMIT: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CapabilityManifest {
    pub schema: u8,
    pub peer: &'static str,
    pub interactive: &'static [&'static str],
    pub headless: &'static [&'static str],
}

pub const CAPABILITIES: CapabilityManifest = CapabilityManifest {
    schema: 1,
    peer: "tellus",
    interactive: &[
        "tellus.observe.v1",
        "tellus.verify-grounding.v1",
        "tellus.sme-plan.v1",
    ],
    headless: &[
        "tellus.observe.v1",
        "tellus.verify-grounding.v1",
        "tellus.sme-plan.v1",
    ],
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryV1 {
    pub load_id: String,
    pub state: String,
    pub freshness: String,
    pub sha256: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservationV1 {
    pub complete: bool,
    pub status: String,
    pub entries: Vec<EntryV1>,
    pub truncated: bool,
}

pub trait TellusPort {
    type Error;
    fn observe(&mut self, limit: usize) -> Result<ObservationV1, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroundingDepthV1 {
    Ambient,
    Activated,
    Reference,
}

impl GroundingDepthV1 {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ambient => "ambient",
            Self::Activated => "activated",
            Self::Reference => "reference",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroundingSelectionV1 {
    pub member: [u8; 16],
    pub authority_sha256: [u8; 32],
    pub surface_sha256: [u8; 32],
    pub depth: GroundingDepthV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GroundingVerificationRequestV1 {
    pub method_sha256: [u8; 32],
    pub fold_sha256: [u8; 32],
    pub orthogonality_sha256: [u8; 32],
    pub selections: Vec<GroundingSelectionV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedGroundingSourceV1 {
    pub load_id: String,
    pub sha256: [u8; 32],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedGroundingSourcesV1 {
    pub method: VerifiedGroundingSourceV1,
    pub fold: VerifiedGroundingSourceV1,
    pub orthogonality: VerifiedGroundingSourceV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroundingStaleReasonV1 {
    SourceMissing,
    SourceChanged,
    GroundingMissing,
    GroundingMismatch,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GroundingVerificationV1 {
    Current {
        sources: VerifiedGroundingSourcesV1,
        members: Vec<[u8; 16]>,
    },
    Stale(GroundingStaleReasonV1),
}

pub trait GroundingVerificationPort {
    type Error;

    fn verify_grounding(
        &mut self,
        request: &GroundingVerificationRequestV1,
    ) -> Result<GroundingVerificationV1, Self::Error>;
}

pub const SME_ENSEMBLE_QUESTION: &str = "do we have the right ensemble?";
pub const SME_ADVISORY_BOUNDARY: &str = "SME routing and retrieval are advisory evidence, not stakeholder equity, independence, approval, authority, or proof";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmePlanRequestV1 {
    pub fold_load_id: String,
    pub lattice_load_id: String,
    pub tags: Vec<String>,
    pub depth: GroundingDepthV1,
    pub max_roles: u8,
    pub max_surface_bytes: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmePlanStatusV1 {
    Current,
    ReviewRequired,
    Unmatched,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmePlanSourceV1 {
    pub load_id: String,
    pub sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SmeEntryKindV1 {
    Guidance,
    Question,
    Hazard,
    Probe,
    Reference,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmeEntryV1 {
    pub id: Option<String>,
    pub kind: SmeEntryKindV1,
    pub text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmeRoleV1 {
    pub role: String,
    pub reference: String,
    pub authority_sha256: [u8; 32],
    pub surface_sha256: [u8; 32],
    pub depth: GroundingDepthV1,
    pub surface_version: u8,
    pub entries: Vec<SmeEntryV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmeOrthogonalityPairV1 {
    pub role1: String,
    pub role2: String,
    pub orthogonal: bool,
    pub facet1: String,
    pub facet2: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmeOrthogonalityV1 {
    pub all_orthogonal: bool,
    pub sufficient_roles: bool,
    pub pairs: Vec<SmeOrthogonalityPairV1>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SmePlanV1 {
    pub status: SmePlanStatusV1,
    pub complete: bool,
    pub next_action: String,
    pub fold: SmePlanSourceV1,
    pub lattice: SmePlanSourceV1,
    pub tags: Vec<String>,
    pub roles: Vec<SmeRoleV1>,
    pub orthogonality: SmeOrthogonalityV1,
}

pub trait SmePlanPort {
    type Error;

    fn plan_smes(&mut self, request: &SmePlanRequestV1) -> Result<SmePlanV1, Self::Error>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Input {
    Refresh,
    Close,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Running,
    CloseRequested,
}

pub fn translate_key(key: KeyEvent) -> Option<Input> {
    if key.kind != KeyEventKind::Press || key.modifiers != KeyModifiers::NONE {
        return None;
    }
    match key.code {
        KeyCode::Char('r') => Some(Input::Refresh),
        KeyCode::Char('q') => Some(Input::Close),
        _ => None,
    }
}

struct WorkerReply(Result<ObservationV1, String>);

pub struct Component {
    refresh: SyncSender<()>,
    replies: Receiver<WorkerReply>,
    pending: bool,
    observation: Option<ObservationV1>,
    notice: String,
}

impl Component {
    pub fn new<P>(mut port: P) -> Self
    where
        P: TellusPort + Send + 'static,
        P::Error: Send + 'static,
    {
        let (tx, rx) = sync_channel(1);
        let (reply_tx, reply_rx) = sync_channel(1);
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                let result = port
                    .observe(OBSERVATION_LIMIT)
                    .map_err(|_| "Tellus observation unavailable".to_owned());
                if reply_tx.send(WorkerReply(result)).is_err() {
                    break;
                }
            }
        });
        let mut component = Self {
            refresh: tx,
            replies: reply_rx,
            pending: false,
            observation: None,
            notice: "Tellus ready; refresh queued".into(),
        };
        component.offer();
        component
    }

    pub fn poll(&mut self) {
        let _ = self.poll_settlement();
    }

    pub fn poll_settlement(&mut self) -> Option<Result<(), ()>> {
        let mut settlement = None;
        loop {
            match self.replies.try_recv() {
                Ok(WorkerReply(Ok(value))) => {
                    self.pending = false;
                    self.observation = Some(value);
                    self.notice = "Current Tellus observation".into();
                    settlement = Some(Ok(()));
                }
                Ok(WorkerReply(Err(status))) => {
                    self.pending = false;
                    self.notice = status;
                    settlement = Some(Err(()));
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if self.pending {
                        settlement = Some(Err(()));
                    }
                    self.pending = false;
                    self.notice = "Tellus worker unavailable".into();
                    break;
                }
            }
        }
        settlement
    }

    pub const fn request_pending(&self) -> bool {
        self.pending
    }

    pub fn update(&mut self, input: Input, focused: bool) -> Lifecycle {
        if !focused {
            return Lifecycle::Running;
        }
        match input {
            Input::Close => Lifecycle::CloseRequested,
            Input::Refresh => {
                self.offer();
                Lifecycle::Running
            }
        }
    }

    pub fn draw_in(&self, frame: &mut Frame<'_>, area: Rect, focused: bool) {
        let [body, footer] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(area);
        let title = if focused {
            " Tellus • focused "
        } else {
            " Tellus "
        };
        let lines = self.observation.as_ref().map_or_else(
            || vec![Line::from("No current Tellus observation")],
            |value| {
                let mut lines = vec![Line::from(format!(
                    "{} load(s) • status {}{}",
                    value.entries.len(),
                    value.status,
                    if value.truncated {
                        " • truncated"
                    } else {
                        ""
                    }
                ))];
                for entry in &value.entries {
                    let identity = entry.sha256.as_deref().map_or_else(
                        || "no current identity".to_owned(),
                        |digest| digest.chars().take(12).collect(),
                    );
                    lines.push(Line::from(format!(
                        "{} • {} • {} • {}",
                        entry.load_id, entry.state, entry.freshness, identity
                    )));
                }
                lines
            },
        );
        frame.render_widget(
            Paragraph::new(lines).block(Block::default().title(title).borders(Borders::ALL)),
            body,
        );
        frame.render_widget(Paragraph::new(self.notice.as_str()), footer);
    }

    fn offer(&mut self) {
        if self.pending {
            self.notice = "Tellus refresh already pending".into();
        } else if self.refresh.try_send(()).is_ok() {
            self.pending = true;
            self.notice = "Tellus refresh pending".into();
        } else {
            self.notice = "Tellus worker unavailable".into();
        }
    }
}

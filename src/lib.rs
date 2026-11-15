//! Composable Grounding presentation over a typed, non-MCP client boundary.
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
    peer: "grounding",
    interactive: &["grounding.observe.v1"],
    headless: &["grounding.observe.v1"],
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

pub trait GroundingPort {
    type Error;
    fn observe(&mut self, limit: usize) -> Result<ObservationV1, Self::Error>;
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
        P: GroundingPort + Send + 'static,
        P::Error: Send + 'static,
    {
        let (tx, rx) = sync_channel(1);
        let (reply_tx, reply_rx) = sync_channel(1);
        std::thread::spawn(move || {
            while rx.recv().is_ok() {
                let result = port
                    .observe(OBSERVATION_LIMIT)
                    .map_err(|_| "Grounding observation unavailable".to_owned());
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
            notice: "Grounding ready; refresh queued".into(),
        };
        component.offer();
        component
    }

    pub fn poll(&mut self) {
        loop {
            match self.replies.try_recv() {
                Ok(WorkerReply(Ok(value))) => {
                    self.pending = false;
                    self.observation = Some(value);
                    self.notice = "Current Grounding observation".into();
                }
                Ok(WorkerReply(Err(status))) => {
                    self.pending = false;
                    self.notice = status;
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    self.pending = false;
                    self.notice = "Grounding worker unavailable".into();
                    break;
                }
            }
        }
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
            " Grounding • focused "
        } else {
            " Grounding "
        };
        let lines = self.observation.as_ref().map_or_else(
            || vec![Line::from("No current Grounding observation")],
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
            self.notice = "Grounding refresh already pending".into();
        } else if self.refresh.try_send(()).is_ok() {
            self.pending = true;
            self.notice = "Grounding refresh pending".into();
        } else {
            self.notice = "Grounding worker unavailable".into();
        }
    }
}

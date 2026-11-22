use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tagrex_service::Service;
use uuid::Uuid;

pub(crate) const HEARTBEAT: Duration = Duration::from_secs(4);
const EXPIRES: Duration = Duration::from_secs(13);
const MAX_SESSIONS: usize = 64;

#[derive(Clone, PartialEq, Eq, Serialize)]
struct Agent {
    id: String,
    principal: String,
    name: String,
    model: Option<String>,
    state: SessionState,
    controllable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SessionState {
    Active,
    Paused,
    Disconnected,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionControl {
    Pause,
    Resume,
    Disconnect,
    Reconnect,
}

struct Session {
    agent: Agent,
    last_seen: Instant,
    expired: bool,
}

#[derive(Clone, Serialize)]
struct Work {
    principal: String,
    session: Option<String>,
    kind: &'static str,
    tool: String,
}

#[derive(Serialize)]
struct Staged {
    id: String,
    files: u64,
    principal: String,
    session: Option<String>,
}

#[derive(Default)]
pub(crate) struct Activity(Mutex<State>);

#[derive(Default)]
struct State {
    sessions: HashMap<Uuid, Session>,
    work: Vec<(Uuid, Work)>,
    operations: VecDeque<(String, Uuid)>,
    staged: Option<(Instant, Staged)>,
}

fn short_label(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(96)
        .collect()
}

impl Activity {
    pub(crate) fn heartbeat(
        &self,
        principal: &str,
        session: Uuid,
        name: &str,
        model: Option<&str>,
        connected: bool,
        controllable: bool,
    ) -> Option<bool> {
        let mut state = self.0.lock().ok()?;
        let previous = state.sessions.get(&session);
        if previous.is_some_and(|entry| entry.agent.principal != principal) {
            return None;
        }
        if !connected {
            return Some(state.sessions.remove(&session).is_some());
        }
        let agent = Agent {
            id: session.to_string(),
            principal: principal.into(),
            name: short_label(name),
            model: model.map(short_label).filter(|value| !value.is_empty()),
            state: previous.map_or(SessionState::Active, |entry| entry.agent.state),
            controllable,
        };
        let changed = previous.is_none_or(|entry| entry.expired || entry.agent != agent);
        if previous.is_none() && state.sessions.len() >= MAX_SESSIONS {
            let oldest = state
                .sessions
                .iter()
                .filter(|(_, entry)| entry.expired)
                .min_by_key(|(_, entry)| entry.last_seen)
                .map(|(id, _)| *id)?;
            state.sessions.remove(&oldest);
        }
        state.sessions.insert(
            session,
            Session {
                agent,
                last_seen: Instant::now(),
                expired: false,
            },
        );
        Some(changed)
    }

    pub(crate) fn control(&self, id: Uuid, action: SessionControl) -> bool {
        let Ok(mut state) = self.0.lock() else {
            return false;
        };
        let Some(entry) = state.sessions.get_mut(&id) else {
            return false;
        };
        if !entry.agent.controllable || entry.expired {
            return false;
        }
        entry.agent.state = match (entry.agent.state, action) {
            (SessionState::Active, SessionControl::Pause) => SessionState::Paused,
            (SessionState::Paused, SessionControl::Resume)
            | (SessionState::Disconnected, SessionControl::Reconnect) => SessionState::Active,
            (SessionState::Active | SessionState::Paused, SessionControl::Disconnect) => {
                SessionState::Disconnected
            }
            _ => return false,
        };
        true
    }

    pub(crate) fn denied(
        &self,
        session: Uuid,
        principal: &str,
        tool: &str,
    ) -> Option<(&'static str, &'static str)> {
        let Ok(state) = self.0.lock() else {
            return Some(("agent_unavailable", "Agent controls are unavailable."));
        };
        let entry = state.sessions.get(&session)?;
        if entry.agent.principal != principal {
            return Some((
                "unauthorized",
                "The agent connection does not own this session.",
            ));
        }
        match entry.agent.state {
            SessionState::Disconnected => Some((
                "agent_disconnected",
                "Reconnect this agent in TAGnetic to continue.",
            )),
            SessionState::Paused if !read_only_while_paused(tool) => Some((
                "agent_paused",
                "Edits are paused for this agent in TAGnetic.",
            )),
            _ => None,
        }
    }

    pub(crate) fn annotate_writes(&self, writes: &mut [Value]) {
        let Ok(state) = self.0.lock() else { return };
        for write in writes {
            let Some(id) = write["id"].as_str() else {
                continue;
            };
            if let Some((_, session)) = state
                .operations
                .iter()
                .find(|(operation, _)| operation == id)
            {
                write["session"] = Value::String(session.to_string());
            }
        }
    }

    pub(crate) fn expire(&self) -> bool {
        let Ok(mut state) = self.0.lock() else {
            return false;
        };
        let mut changed = false;
        state.sessions.retain(|_, entry| {
            if entry.last_seen.elapsed() < EXPIRES {
                return true;
            }
            changed |= !entry.expired;
            entry.expired = true;
            // A delayed heartbeat must retain an explicit pause or disconnect.
            entry.agent.state != SessionState::Active
        });
        changed
    }

    pub(crate) fn snapshot(&self) -> Value {
        let Ok(state) = self.0.lock() else {
            return Value::Null;
        };
        let mut connections: Vec<_> = state
            .sessions
            .values()
            .filter(|entry| !entry.expired)
            .map(|entry| &entry.agent)
            .collect();
        connections.sort_by(|a, b| (&a.name, &a.model, &a.id).cmp(&(&b.name, &b.model, &b.id)));
        let staged = state
            .staged
            .as_ref()
            .filter(|(time, _)| time.elapsed() < Duration::from_secs(3));
        serde_json::json!({
            "connections":connections,
            "work":state.work.iter().map(|(_, work)| work).collect::<Vec<_>>(),
            "staged":staged.map(|(_, receipt)| receipt),
        })
    }

    pub(crate) fn start(
        self: &Arc<Self>,
        service: &Arc<Service>,
        principal: &str,
        session: Option<Uuid>,
        tool: &str,
    ) -> Option<ActiveCall> {
        let kind = match tool {
            "plan.commit" | "files.write" => "saving",
            "draft.get" | "draft.discard" => return None,
            _ if tool.starts_with("draft.") => "staging",
            "files.query" | "files.snapshot" | "providers.search" | "providers.release" => {
                "searching"
            }
            _ => return None,
        };
        let id = Uuid::new_v4();
        self.0.lock().ok()?.work.push((
            id,
            Work {
                principal: principal.into(),
                session: session.map(|value| value.to_string()),
                kind,
                tool: tool.into(),
            },
        ));
        service.notify_agent_activity();
        Some(ActiveCall {
            id,
            activity: self.clone(),
            service: service.clone(),
            staged: None,
            operation: None,
        })
    }
}

pub(crate) struct ActiveCall {
    id: Uuid,
    activity: Arc<Activity>,
    service: Arc<Service>,
    staged: Option<u64>,
    operation: Option<String>,
}

impl ActiveCall {
    pub(crate) fn completed(&mut self, value: &Value) {
        self.staged = value
            .get("files")
            .and_then(Value::as_u64)
            .filter(|files| *files > 0);
        self.operation = value
            .get("operation")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }
}

impl Drop for ActiveCall {
    fn drop(&mut self) {
        if let Ok(mut state) = self.activity.0.lock() {
            if let Some(index) = state.work.iter().position(|(id, _)| *id == self.id) {
                let (_, work) = state.work.remove(index);
                if let (Some(operation), Some(session)) = (
                    self.operation.take(),
                    work.session
                        .as_deref()
                        .and_then(|id| Uuid::parse_str(id).ok()),
                ) {
                    if !state.operations.iter().any(|(id, _)| id == &operation) {
                        if state.operations.len() >= 128 {
                            state.operations.pop_front();
                        }
                        state.operations.push_back((operation, session));
                    }
                }
                if let Some(files) = self.staged.filter(|_| work.kind == "staging") {
                    state.staged = Some((
                        Instant::now(),
                        Staged {
                            id: self.id.to_string(),
                            files,
                            principal: work.principal,
                            session: work.session,
                        },
                    ));
                }
            }
        }
        self.service.notify_agent_activity();
    }
}

fn read_only_while_paused(tool: &str) -> bool {
    matches!(
        tool,
        "session.status"
            | "schema.catalog"
            | "files.snapshot"
            | "files.page"
            | "files.query"
            | "files.select"
            | "files.read"
            | "files.field"
            | "files.locks"
            | "draft.get"
            | "staging.get"
            | "operations.status"
            | "operations.cancel"
            | "queries.cancel"
            | "history.list"
            | "settings.get"
            | "presets.list"
            | "providers.status"
            | "providers.search"
            | "providers.release"
            | "artwork.inspect"
            | "artwork.read"
            | "artwork.embedded"
            | "masks.render"
            | "playback.status"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_are_independent_for_sessions_with_the_same_pairing() {
        let activity = Activity::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        for session in [first, second] {
            assert_eq!(
                activity.heartbeat("paired", session, "Codex", None, true, true),
                Some(true)
            );
        }

        assert!(activity.control(first, SessionControl::Pause));
        assert_eq!(
            activity.denied(first, "paired", "draft.create").unwrap().0,
            "agent_paused"
        );
        assert!(activity.denied(first, "paired", "files.read").is_none());
        assert!(activity.denied(second, "paired", "draft.create").is_none());
        assert_eq!(
            activity.denied(second, "other", "files.read").unwrap().0,
            "unauthorized"
        );

        assert!(activity.control(first, SessionControl::Disconnect));
        assert_eq!(
            activity.denied(first, "paired", "files.read").unwrap().0,
            "agent_disconnected"
        );
        assert!(activity.denied(second, "paired", "files.read").is_none());
        assert!(activity.control(first, SessionControl::Reconnect));
        assert!(activity.denied(first, "paired", "draft.create").is_none());
    }
}

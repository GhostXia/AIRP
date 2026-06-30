//! Broadcast bus: connects dispatch results to SSE subscriber streams.
//!
//! Each SSE connection holds a receiver. The dispatcher sends downstream
//! [`Envelope`]s via the sender. Connections filter by their subscribed scopes
//! (set via the `subscribe` intent); if a connection has no scope filter, it
//! receives everything.
//!
//! Connection ids are strings (a client-supplied id via the `?conn=` query, or
//! a server-generated UUID). The `subscriptions` map is a *sync* mutex so the
//! SSE handler's drop guard can clean up a connection synchronously when the
//! client disconnects — avoiding an unbounded leak of stale scope filters.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use super::envelope::Envelope;

/// How many envelopes to buffer per broadcast channel. Old envelopes are
/// dropped when the buffer is full (slow consumers lose messages — acceptable
/// for a real-time UI where stale state is worse than a gap).
const BROADCAST_CAPACITY: usize = 256;

/// Shared between the dispatch handler (sender) and every SSE stream (receiver).
#[derive(Debug, Clone)]
pub struct Bus {
    tx: broadcast::Sender<Envelope>,
    /// Per-connection scope subscriptions, keyed by connection id.
    subscriptions: Arc<Mutex<HashMap<String, Vec<String>>>>,
}

impl Bus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            tx,
            subscriptions: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Register a new connection. Returns `(generated_conn_id, receiver)`.
    /// The id is a fresh UUID; the SSE handler may instead use a client-supplied
    /// id from the `?conn=` query so that later `subscribe` intents correlate.
    pub fn subscribe(&self) -> (String, broadcast::Receiver<Envelope>) {
        let conn_id = uuid::Uuid::new_v4().to_string();
        (conn_id, self.tx.subscribe())
    }

    /// Set the scope filter for a connection. Replaces the previous set.
    /// Empty vector = receive all scopes (no filter).
    pub fn set_scopes(&self, conn_id: &str, scopes: Vec<String>) {
        let mut map = self.subscriptions.lock().unwrap();
        if scopes.is_empty() {
            map.remove(conn_id);
        } else {
            map.insert(conn_id.to_string(), scopes);
        }
    }

    /// Remove a connection's scope filter when its SSE stream ends.
    pub fn disconnect(&self, conn_id: &str) {
        self.subscriptions.lock().unwrap().remove(conn_id);
    }

    /// Broadcast an envelope to all connections.
    ///
    /// `broadcast::Sender` does not support per-receiver filtering, so the
    /// envelope is sent to every subscriber and each SSE stream filters on its
    /// own side via [`Bus::wants_scope`]. The channel is bounded; slow consumers lag.
    pub async fn send(&self, envelope: Envelope) {
        let _ = self.tx.send(envelope);
    }

    /// Whether a connection wants envelopes for a given scope. True if the
    /// connection has no filter, the envelope is non-scoped, or the scope matches.
    pub fn wants_scope(&self, conn_id: &str, scope: Option<&str>) -> bool {
        let map = self.subscriptions.lock().unwrap();
        match map.get(conn_id) {
            None => true, // no filter = receive all
            Some(scopes) => match scope {
                None => true, // non-scope envelope (blueprint/manifest/error) = always
                Some(s) => scopes.iter().any(|fs| fs == s),
            },
        }
    }
}

impl Default for Bus {
    fn default() -> Self {
        Self::new()
    }
}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Session {
    pub id: String,
    pub title: String,
    pub cwd: String,
    pub model: String,
    pub status: String,
    pub started_at: String,
    pub last_event_at: String,
    pub ended_at: Option<String>,
    pub parent_type: Option<String>,
    pub parent_id: Option<String>,
    pub sort_order: i64,
    pub waiting_tool: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Todo {
    pub id: i64,
    pub text: String,
    pub note: String,
    pub status: String,
    pub sort_order: i64,
    pub created_by_session: Option<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
    pub parent_type: Option<String>,
    pub parent_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CardId {
    Session(String),
    Todo(i64),
}

impl CardId {
    /// Parse from prefixed string: "s:abc" or "t:42"
    pub fn from_prefixed(s: &str) -> Option<Self> {
        if let Some(id) = s.strip_prefix("s:") {
            Some(CardId::Session(id.to_string()))
        } else if let Some(id) = s.strip_prefix("t:") {
            id.parse::<i64>().ok().map(CardId::Todo)
        } else {
            None
        }
    }

    /// Convert to prefixed string for HTML data attributes
    pub fn to_prefixed(&self) -> String {
        match self {
            CardId::Session(id) => format!("s:{id}"),
            CardId::Todo(id) => format!("t:{id}"),
        }
    }

    /// Convert to (parent_type, parent_id) pair for DB storage
    pub fn to_db_pair(&self) -> (&'static str, String) {
        match self {
            CardId::Session(id) => ("session", id.clone()),
            CardId::Todo(id) => ("todo", id.to_string()),
        }
    }

    /// Build from DB parent_type/parent_id columns
    pub fn from_db(parent_type: &str, parent_id: &str) -> Option<Self> {
        match parent_type {
            "session" => Some(CardId::Session(parent_id.to_string())),
            "todo" => parent_id.parse::<i64>().ok().map(CardId::Todo),
            _ => None,
        }
    }
}

impl fmt::Display for CardId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_prefixed())
    }
}

/// Build a card tree from sessions and todos.
/// Returns (root_ids, children_map) where children_map maps each parent to its children.
pub fn build_card_tree(sessions: &[Session], todos: &[Todo]) -> (Vec<CardId>, HashMap<CardId, Vec<CardId>>) {
    let mut children_map: HashMap<CardId, Vec<CardId>> = HashMap::new();
    let mut roots = Vec::new();

    // Collect all valid card IDs for orphan detection
    let mut all_ids: std::collections::HashSet<CardId> = std::collections::HashSet::new();
    for s in sessions {
        all_ids.insert(CardId::Session(s.id.clone()));
    }
    for t in todos {
        all_ids.insert(CardId::Todo(t.id));
    }

    // Helper: classify a card as root or child
    let mut place_card = |card_id: CardId, parent_type: &Option<String>, parent_id: &Option<String>| {
        if let (Some(pt), Some(pid)) = (parent_type, parent_id) {
            if let Some(parent) = CardId::from_db(pt, pid) {
                if all_ids.contains(&parent) {
                    children_map.entry(parent).or_default().push(card_id);
                    return;
                }
            }
        }
        // No parent or orphaned → root
        roots.push(card_id);
    };

    // Merge sessions and todos into a single list sorted by (is_done, sort_order)
    // so they interleave correctly at every level
    enum Card<'a> {
        S(&'a Session),
        T(&'a Todo),
    }
    let mut all_cards: Vec<Card> = Vec::new();
    for s in sessions {
        all_cards.push(Card::S(s));
    }
    for t in todos {
        all_cards.push(Card::T(t));
    }
    all_cards.sort_by_key(|c| match c {
        Card::S(s) => (false, s.sort_order),
        Card::T(t) => (t.status == "done", t.sort_order),
    });

    for c in &all_cards {
        match c {
            Card::S(s) => place_card(CardId::Session(s.id.clone()), &s.parent_type, &s.parent_id),
            Card::T(t) => place_card(CardId::Todo(t.id), &t.parent_type, &t.parent_id),
        }
    }

    (roots, children_map)
}

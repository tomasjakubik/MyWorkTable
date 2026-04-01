use sqlx::SqlitePool;
use tokio::sync::broadcast;

#[derive(Debug, Clone)]
pub enum AppEvent {
    SessionUpdated,
    TodoUpdated,
    Sound(&'static str),
}

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub events_tx: broadcast::Sender<AppEvent>,
}

impl AppState {
    pub fn new(db: SqlitePool) -> Self {
        let (events_tx, _) = broadcast::channel(512);
        Self { db, events_tx }
    }
}

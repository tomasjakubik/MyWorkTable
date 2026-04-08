use sqlx::SqlitePool;
use sqlx::sqlite::SqlitePoolOptions;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::models::{Session, Todo};

pub async fn init_db() -> SqlitePool {
    let db_dir = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("my-work-table");
    std::fs::create_dir_all(&db_dir).expect("Failed to create data directory");

    let db_path = db_dir.join("data.db");
    let url = format!("sqlite:{}?mode=rwc", db_path.display());

    let pool = SqlitePoolOptions::new()
        .max_connections(20)
        .connect(&url)
        .await
        .expect("Failed to connect to database");

    // Enable WAL mode
    sqlx::query("PRAGMA journal_mode=WAL")
        .execute(&pool)
        .await
        .expect("Failed to enable WAL mode");

    // Run migrations
    let migration_sql = include_str!("../migrations/001_initial.sql");
    for statement in migration_sql.split(';') {
        let trimmed = statement.trim();
        if !trimmed.is_empty() {
            sqlx::query(trimmed)
                .execute(&pool)
                .await
                .expect("Failed to run migration");
        }
    }

    // Run incremental migrations (ALTER TABLE fails silently if already applied)
    for migration in [
        include_str!("../migrations/002_waiting_tool.sql"),
        include_str!("../migrations/003_observer_features.sql"),
    ] {
        for statement in migration.split(';') {
            let trimmed = statement.trim();
            if !trimmed.is_empty() {
                sqlx::query(trimmed).execute(&pool).await.ok();
            }
        }
    }

    pool
}

pub async fn get_sessions(pool: &SqlitePool) -> Vec<Session> {
    sqlx::query_as::<_, Session>("SELECT * FROM sessions ORDER BY last_event_at DESC")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

pub async fn get_todos(pool: &SqlitePool) -> Vec<Todo> {
    sqlx::query_as::<_, Todo>("SELECT * FROM todos ORDER BY (status = 'done') ASC, sort_order ASC, created_at DESC")
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Returns (done_count, total_count) per session_id.
pub async fn get_task_counts(pool: &SqlitePool) -> HashMap<String, (i64, i64)> {
    sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT session_id, SUM(status = 'done'), COUNT(*) FROM session_tasks GROUP BY session_id",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(sid, done, total)| (sid, (done, total)))
    .collect()
}

pub async fn delete_all_data(pool: &SqlitePool) {
    sqlx::query("DELETE FROM session_tasks").execute(pool).await.ok();
    sqlx::query("DELETE FROM events").execute(pool).await.ok();
    sqlx::query("DELETE FROM todos").execute(pool).await.ok();
    sqlx::query("DELETE FROM sessions").execute(pool).await.ok();
}

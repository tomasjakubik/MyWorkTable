use askama::Template;
use axum::{
    Json, Router,
    extract::{ConnectInfo, Path, State},
    http::StatusCode,
    middleware,
    response::{
        Html, IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{delete, get, post},
};
use std::net::SocketAddr;
use futures::stream::Stream;
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt::Write as FmtWrite;
use tower_http::cors::CorsLayer;
use tower_http::services::ServeDir;

use crate::db;
use crate::models::{CardId, Session, Todo, build_card_tree};
use crate::state::{AppEvent, AppState};
use crate::time::relative_time;

/// Only allow connections from loopback and Docker/private subnets.
async fn require_local(
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: axum::extract::Request,
    next: middleware::Next,
) -> impl IntoResponse {
    let ip = addr.ip();
    let allowed = ip.is_loopback()
        || match ip {
            std::net::IpAddr::V4(v4) => {
                let octets = v4.octets();
                // 172.16.0.0/12 — Docker bridge/custom networks
                octets[0] == 172 && (octets[1] & 0xF0) == 16
            }
            std::net::IpAddr::V6(_) => false,
        };
    if allowed {
        next.run(request).await.into_response()
    } else {
        StatusCode::FORBIDDEN.into_response()
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // HTML routes
        .route("/", get(index_page))
        .route("/html/cards", get(html_cards))
        .route("/html/todo/{id}/edit-text", get(html_edit_text))
        .route("/html/todo/{id}/edit-note", get(html_edit_note))
        // JSON API
        .route("/health", get(health))
        .route("/hooks/{event_type}", post(handle_hook))
        .route("/api/events", get(sse_events))
        .route("/api/sessions", get(get_sessions).delete(clear_sessions))
        .route("/api/todos", get(get_todos).delete(clear_todos))
        .route("/api/todos", post(create_todo))
        .route("/api/todos/{id}", delete(delete_todo))
        .route("/api/todos/{id}/update", post(update_todo))
        .route("/api/todos/{id}/update-json", post(update_todo_json))
        .route("/api/todos/{id}/done", post(complete_todo))
        .route("/api/todos/{id}/toggle", post(toggle_todo))
        .route("/api/sessions/{id}", delete(delete_session).post(update_session))
        .route("/api/cards/move", post(move_card))
        .route("/api/settings/{key}", get(get_setting).post(set_setting))
        .route("/api/database", delete(delete_database))
        // Static files
        .nest_service("/assets", ServeDir::new("assets"))
        .layer(CorsLayer::permissive())
        .layer(middleware::from_fn(require_local))
        .with_state(state)
}

// --- Markdown ---

pub fn render_markdown(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    let parser = pulldown_cmark::Parser::new(input);
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, parser);
    html
}

// --- HTML templates ---

#[derive(Template)]
#[template(path = "index.html")]
struct IndexTemplate;

async fn index_page() -> Html<String> {
    Html(IndexTemplate.render().unwrap())
}

// View models with precomputed display fields

struct SessionView {
    id: String,
    title: String,
    cwd: String,
    model: String,
    last_event_at: String,
    card_class: &'static str,
    badge_class: &'static str,
    status_label: String,
    dir_name: String,
    rel_time: String,
}

impl From<Session> for SessionView {
    fn from(s: Session) -> Self {
        let (card_class, badge_class, status_label) = match s.status.as_str() {
            "active" => (
                "bg-amber-50 border-amber-200 text-amber-900",
                "bg-amber-200 text-amber-700",
                "working".to_string(),
            ),
            "waiting" => {
                let label = match &s.waiting_tool {
                    Some(tool) if !tool.is_empty() => format!("approve {tool}"),
                    _ => "waiting for approval".to_string(),
                };
                (
                    "bg-blue-50 border-blue-200 text-blue-900",
                    "bg-blue-200 text-blue-700",
                    label,
                )
            }
            "ended" => (
                "bg-green-50 border-green-200 text-green-900",
                "bg-green-200 text-green-700",
                "ended".to_string(),
            ),
            _ => (
                "bg-gray-50 border-gray-200 text-gray-700",
                "bg-gray-200 text-gray-600",
                "unknown".to_string(),
            ),
        };
        let dir_name = s.cwd.rsplit('/').next().unwrap_or(&s.cwd).to_string();
        let rel_time = relative_time(&s.last_event_at);
        SessionView {
            id: s.id,
            title: s.title,
            cwd: s.cwd,
            model: s.model,
            last_event_at: s.last_event_at,
            card_class,
            badge_class,
            status_label,
            dir_name,
            rel_time,
        }
    }
}

struct TodoView {
    id: i64,
    text: String,
    note: String,
    note_html: String,
    is_done: bool,
}

impl From<Todo> for TodoView {
    fn from(t: Todo) -> Self {
        let note_html = render_markdown(&t.note);
        let is_done = t.status == "done";
        TodoView {
            id: t.id,
            text: t.text,
            note: t.note,
            note_html,
            is_done,
        }
    }
}

// --- Card tree rendering ---

async fn html_cards(State(state): State<AppState>) -> Html<String> {
    let sessions = db::get_sessions(&state.db).await;
    let todos = db::get_todos(&state.db).await;

    let session_views: HashMap<String, SessionView> = sessions
        .iter()
        .cloned()
        .map(|s| (s.id.clone(), SessionView::from(s)))
        .collect();
    let todo_views: HashMap<i64, TodoView> = todos
        .iter()
        .cloned()
        .map(|t| (t.id, TodoView::from(t)))
        .collect();

    let (roots, children_map) = build_card_tree(&sessions, &todos);

    let mut html = String::new();

    // New todo input
    html.push_str(r#"<div class="mb-4 flex gap-2"><input id="new-todo-input" class="flex-1 bg-gray-800 border border-gray-600 rounded-lg px-3 py-2 text-sm text-gray-200 placeholder-gray-500 focus:outline-none focus:border-blue-500 transition-colors" placeholder="Add a todo..."><button class="bg-gray-700 hover:bg-red-600 text-gray-300 hover:text-white text-xs px-3 py-2 rounded-lg border border-gray-600 hover:border-red-500 transition-colors flex-shrink-0" onclick="handleClearAll(this)" data-confirmed="false">Clear all</button></div>"#);

    // Root card list
    html.push_str(r#"<div id="card-list" class="card-children" style="display:flex;flex-direction:column;gap:0.5rem" data-parent="root">"#);
    render_card_list(&mut html, &roots, &children_map, &session_views, &todo_views);
    html.push_str("</div>");

    Html(html)
}

fn render_card_list(
    html: &mut String,
    items: &[CardId],
    children_map: &HashMap<CardId, Vec<CardId>>,
    session_views: &HashMap<String, SessionView>,
    todo_views: &HashMap<i64, TodoView>,
) {
    for card_id in items {
        let prefixed = card_id.to_prefixed();
        let children = children_map.get(card_id).map(|v| v.as_slice()).unwrap_or(&[]);
        let has_children = !children.is_empty();

        match card_id {
            CardId::Session(id) => {
                if let Some(s) = session_views.get(id) {
                    render_session_card(html, s, &prefixed, has_children);
                    // Children container
                    let _ = write!(html, r#"<div class="card-children" style="margin-left:1rem;padding-left:0.5rem;margin-top:2px;border-left:2px solid #374151;display:flex;flex-direction:column;gap:2px" data-parent="{prefixed}">"#);
                    render_card_list(html, children, children_map, session_views, todo_views);
                    html.push_str("</div></div>");
                }
            }
            CardId::Todo(id) => {
                if let Some(t) = todo_views.get(id) {
                    render_todo_card(html, t, &prefixed, has_children);
                    // Children container
                    let _ = write!(html, r#"<div class="card-children" style="margin-left:1rem;padding-left:0.5rem;margin-top:2px;border-left:2px solid #374151;display:flex;flex-direction:column;gap:2px" data-parent="{prefixed}">"#);
                    render_card_list(html, children, children_map, session_views, todo_views);
                    html.push_str("</div></div>");
                }
            }
        }
    }
}

fn render_session_card(html: &mut String, s: &SessionView, prefixed: &str, _has_children: bool) {
    let _ = write!(
        html,
        concat!(
            r#"<div class="card-item" data-card-id="{prefixed}">"#,
            r#"<div class="rounded-xl p-4 border shadow-sm relative {card_class}">"#,
            r#"<button class="absolute top-2 right-2 w-6 h-6 flex items-center justify-center rounded-full hover:bg-red-100 text-gray-400 hover:text-red-500 text-sm transition-colors" hx-delete="/api/sessions/{id}" hx-swap="none">&#x2715;</button>"#,
            r#"<div class="flex gap-2 pr-6">"#,
            r#"<div class="flex flex-col items-center pt-0.5">"#,
            r#"<span class="drag-handle cursor-grab text-gray-300 hover:text-gray-500 select-none">&#x2630;</span>"#,
            r#"</div>"#,
            r#"<div class="flex-1 min-w-0">"#,
        ),
        prefixed = prefixed,
        card_class = s.card_class,
        id = s.id,
    );
    if !s.title.is_empty() {
        let _ = write!(html, r#"<p class="text-sm font-medium mb-1 truncate">{}</p>"#, html_escape(&s.title));
    }
    let _ = write!(
        html,
        r#"<div class="flex items-center justify-between mb-1"><span class="{name_class} truncate">{dir_name}</span><span class="text-xs px-2 py-0.5 rounded-full flex-shrink-0 {badge_class}">{status_label}</span></div>"#,
        name_class = if !s.title.is_empty() { "text-xs opacity-60" } else { "font-medium text-sm" },
        dir_name = html_escape(&s.dir_name),
        badge_class = s.badge_class,
        status_label = s.status_label,
    );
    let _ = write!(
        html,
        r#"<div class="flex items-center gap-2"><span class="text-xs opacity-60" title="{last_event_at}">{rel_time}</span>"#,
        last_event_at = html_escape(&s.last_event_at),
        rel_time = html_escape(&s.rel_time),
    );
    if !s.model.is_empty() {
        let _ = write!(html, r#"<span class="text-xs opacity-40">{}</span>"#, html_escape(&s.model));
    }
    html.push_str("</div>");
    let _ = write!(html, r#"<div class="text-xs opacity-40 truncate mt-1">{}</div>"#, html_escape(&s.cwd));
    html.push_str("</div></div></div>"); // close flex-1, flex gap-2, rounded card
}

fn render_todo_card(html: &mut String, t: &TodoView, prefixed: &str, _has_children: bool) {
    let card_class = if t.is_done {
        "rounded-xl p-3 border bg-gray-100 border-gray-300 opacity-60"
    } else {
        "rounded-xl p-3 border bg-white border-gray-200 shadow-sm hover:shadow-md transition-shadow"
    };
    let check_class = if t.is_done {
        "bg-green-500 border-green-500 text-white"
    } else {
        "border-gray-400 hover:border-gray-500"
    };
    let _ = write!(
        html,
        concat!(
            r#"<div class="card-item" data-card-id="{prefixed}">"#,
            r#"<div class="{card_class} relative">"#,
            r#"<button class="absolute top-2 right-2 w-6 h-6 flex items-center justify-center rounded-full hover:bg-red-100 text-gray-400 hover:text-red-500 text-sm transition-colors" hx-delete="/api/todos/{id}" hx-swap="none">&#x2715;</button>"#,
            r#"<div class="flex gap-2 pr-6">"#,
            r#"<div class="flex flex-col items-center pt-0.5" style="gap:0.25rem">"#,
            r#"<span class="drag-handle cursor-grab text-gray-300 hover:text-gray-500 select-none">&#x2630;</span>"#,
            r#"<span class="text-xs text-gray-400 font-mono leading-none">#{id}</span>"#,
            r#"<button class="w-4 h-4 rounded border flex-shrink-0 flex items-center justify-center text-xs {check_class}" hx-post="/api/todos/{id}/toggle" hx-swap="none">"#,
        ),
        prefixed = prefixed,
        card_class = card_class,
        id = t.id,
        check_class = check_class,
    );
    if t.is_done {
        html.push_str("&#x2713;");
    }
    html.push_str(r#"</button></div><div class="flex-1 min-w-0">"#);

    // Text
    let text_class = if t.is_done { "text-gray-400 line-through" } else { "text-gray-800" };
    if t.is_done {
        let _ = write!(
            html,
            r#"<p class="text-sm {text_class}" style="cursor: default">{text}</p>"#,
            text_class = text_class,
            text = html_escape(&t.text),
        );
    } else {
        let _ = write!(
            html,
            r#"<p class="text-sm {text_class}" hx-get="/html/todo/{id}/edit-text" hx-trigger="dblclick" hx-target="closest div.flex-1" hx-swap="innerHTML" style="cursor: pointer">{text}</p>"#,
            text_class = text_class,
            id = t.id,
            text = html_escape(&t.text),
        );
    }

    // Note
    if !t.note.is_empty() {
        let note_class = if t.is_done { "text-gray-400" } else { "text-gray-500" };
        if t.is_done {
            let _ = write!(
                html,
                r#"<div class="mt-1 text-xs markdown-body {note_class}">{note_html}</div>"#,
                note_class = note_class,
                note_html = t.note_html,
            );
        } else {
            let _ = write!(
                html,
                r#"<div class="mt-1 text-xs markdown-body {note_class}" hx-get="/html/todo/{id}/edit-note" hx-trigger="click" hx-target="this" hx-swap="outerHTML">{note_html}</div>"#,
                note_class = note_class,
                id = t.id,
                note_html = t.note_html,
            );
        }
    } else if !t.is_done {
        let _ = write!(
            html,
            r#"<button class="text-xs text-gray-400 hover:text-gray-500 transition-colors mt-1" hx-get="/html/todo/{id}/edit-note" hx-trigger="click" hx-target="this" hx-swap="outerHTML">+ add note</button>"#,
            id = t.id,
        );
    }

    html.push_str("</div></div></div>"); // close flex-1, flex gap-2, rounded card
}

/// Strip known context tags and their content from hook input, returning only the
/// user's actual text. Tags like `<ide_opened_file>...</ide_opened_file>` and
/// `<system-reminder>...</system-reminder>` are removed entirely.
fn strip_xml_tags(s: &str) -> String {
    let tags = ["ide_opened_file", "ide_selection", "system-reminder"];
    let mut result = s.to_string();
    for tag in tags {
        loop {
            let open = format!("<{}", tag);
            let close = format!("</{}>", tag);
            let Some(start) = result.find(&open) else { break };
            if let Some(end_pos) = result.find(&close) {
                result.replace_range(start..end_pos + close.len(), "");
            } else {
                // No closing tag — remove from open tag to end
                result.truncate(start);
                break;
            }
        }
    }
    result.trim().to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Template)]
#[template(path = "todo_edit_text.html")]
struct EditTextTemplate {
    id: i64,
    text: String,
}

async fn html_edit_text(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let text: Option<String> = sqlx::query_scalar("SELECT text FROM todos WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    match text {
        Some(text) => Html(EditTextTemplate { id, text }.render().unwrap()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[derive(Template)]
#[template(path = "todo_edit_note.html")]
struct EditNoteTemplate {
    id: i64,
    note: String,
}

async fn html_edit_note(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let note: Option<String> = sqlx::query_scalar("SELECT note FROM todos WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    match note {
        Some(note) => Html(EditNoteTemplate { id, note }.render().unwrap()).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

// --- JSON API ---

async fn health() -> &'static str {
    "ok"
}

// --- SSE ---

async fn sse_events(
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.events_tx.subscribe();
    let stream = async_stream::stream! {
        loop {
            match rx.recv().await {
                Ok(event) => {
                    let name = match event {
                        AppEvent::SessionUpdated | AppEvent::TodoUpdated => "cards",
                    };
                    yield Ok(Event::default().event(name).data("updated"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    yield Ok(Event::default().event("session").data("updated"));
                    yield Ok(Event::default().event("todo").data("updated"));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::default())
}

// --- Hooks ---

#[derive(Debug, Deserialize)]
struct HookPayload {
    session_id: Option<String>,
    tool_name: Option<String>,
    cwd: Option<String>,
    model: Option<String>,
    prompt: Option<String>,
    #[serde(flatten)]
    extra: serde_json::Value,
}

async fn handle_hook(
    State(state): State<AppState>,
    Path(event_type): Path<String>,
    Json(payload): Json<HookPayload>,
) -> impl IntoResponse {
    let session_id = payload
        .session_id
        .unwrap_or_else(|| "unknown".to_string());
    let cwd = payload.cwd.unwrap_or_else(|| ".".to_string());
    let model = payload.model.unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();

    let status = if event_type.contains("Stop") || event_type.contains("End") {
        "ended"
    } else if event_type.contains("PermissionRequest") {
        "waiting"
    } else {
        "active"
    };

    // On PermissionRequest, record which tool is waiting for approval.
    // On any other event, clear it.
    let waiting_tool: Option<&str> = if status == "waiting" {
        payload.tool_name.as_deref()
    } else {
        None
    };

    // Only SessionStart and UserPromptSubmit create new sessions.
    // All other events only update existing ones, so we don't get ghost
    // sessions from events that fire during VSCode startup/shutdown.
    let creates_session = event_type == "SessionStart" || event_type == "UserPromptSubmit";

    if status == "ended" {
        sqlx::query(
            "UPDATE sessions SET last_event_at = ?, status = ?, ended_at = ?, waiting_tool = NULL WHERE id = ?",
        )
        .bind(&now)
        .bind(status)
        .bind(&now)
        .bind(&session_id)
        .execute(&state.db)
        .await
        .ok();
    } else if creates_session {
        sqlx::query(
            "INSERT INTO sessions (id, cwd, model, status, started_at, last_event_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET last_event_at = ?, status = ?, waiting_tool = ?",
        )
        .bind(&session_id)
        .bind(&cwd)
        .bind(&model)
        .bind(status)
        .bind(&now)
        .bind(&now)
        .bind(&now)
        .bind(status)
        .bind(waiting_tool)
        .execute(&state.db)
        .await
        .ok();
    } else {
        sqlx::query(
            "UPDATE sessions SET last_event_at = ?, status = ?, waiting_tool = ? WHERE id = ?",
        )
        .bind(&now)
        .bind(status)
        .bind(waiting_tool)
        .bind(&session_id)
        .execute(&state.db)
        .await
        .ok();
    }

    if event_type == "UserPromptSubmit" {
        if let Some(input) = &payload.prompt {
            let clean = strip_xml_tags(input);
            let title: String = clean.chars().take(80).collect();
            if !title.is_empty() {
                sqlx::query("UPDATE sessions SET title = ? WHERE id = ? AND title = ''")
                    .bind(&title)
                    .bind(&session_id)
                    .execute(&state.db)
                    .await
                    .ok();
            }
        }
    }

    if !model.is_empty() {
        sqlx::query("UPDATE sessions SET model = ? WHERE id = ?")
            .bind(&model)
            .bind(&session_id)
            .execute(&state.db)
            .await
            .ok();
    }

    let metadata = serde_json::to_string(&payload.extra).ok();
    sqlx::query(
        "INSERT INTO events (session_id, event_type, tool_name, timestamp, metadata)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&session_id)
    .bind(&event_type)
    .bind(&payload.tool_name)
    .bind(&now)
    .bind(&metadata)
    .execute(&state.db)
    .await
    .ok();

    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    StatusCode::OK
}

// --- Sessions ---

async fn get_sessions(State(state): State<AppState>) -> impl IntoResponse {
    Json(db::get_sessions(&state.db).await)
}

async fn delete_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    // Read the session's parent (grandparent for reparenting children)
    let parent: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT parent_type, parent_id FROM sessions WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let (gp_type, gp_id) = parent.unwrap_or((None, None));

    // Reparent open child todos to grandparent
    sqlx::query("UPDATE todos SET parent_type = ?, parent_id = ? WHERE parent_type = 'session' AND parent_id = ? AND status != 'done'")
        .bind(&gp_type)
        .bind(&gp_id)
        .bind(&id)
        .execute(&state.db)
        .await
        .ok();
    // Delete done child todos
    sqlx::query("DELETE FROM todos WHERE parent_type = 'session' AND parent_id = ? AND status = 'done'")
        .bind(&id)
        .execute(&state.db)
        .await
        .ok();
    // Reparent child sessions to grandparent
    sqlx::query("UPDATE sessions SET parent_type = ?, parent_id = ? WHERE parent_type = 'session' AND parent_id = ?")
        .bind(&gp_type)
        .bind(&gp_id)
        .bind(&id)
        .execute(&state.db)
        .await
        .ok();

    sqlx::query("DELETE FROM events WHERE session_id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .ok();
    sqlx::query("DELETE FROM sessions WHERE id = ?")
        .bind(&id)
        .execute(&state.db)
        .await
        .ok();
    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

#[derive(Debug, Deserialize)]
struct UpdateSession {
    title: Option<String>,
}

async fn update_session(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSession>,
) -> impl IntoResponse {
    if let Some(title) = &body.title {
        sqlx::query("UPDATE sessions SET title = ? WHERE id = ?")
            .bind(title)
            .bind(&id)
            .execute(&state.db)
            .await
            .ok();
    }
    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    StatusCode::OK
}

async fn clear_sessions(State(state): State<AppState>) -> impl IntoResponse {
    sqlx::query("DELETE FROM events")
        .execute(&state.db)
        .await
        .ok();
    sqlx::query("DELETE FROM sessions")
        .execute(&state.db)
        .await
        .ok();
    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    StatusCode::OK
}

// --- Todos ---

async fn get_todos(State(state): State<AppState>) -> impl IntoResponse {
    Json(db::get_todos(&state.db).await)
}

#[derive(Debug, Deserialize)]
struct CreateTodo {
    text: String,
    note: Option<String>,
    session_id: Option<String>,
}

async fn create_todo(
    State(state): State<AppState>,
    Json(body): Json<CreateTodo>,
) -> impl IntoResponse {
    let note = body.note.unwrap_or_default();
    let (parent_type, parent_id) = if let Some(ref sid) = body.session_id {
        (Some("session".to_string()), Some(sid.clone()))
    } else {
        (None, None)
    };
    // Compute sort_order: one less than the current minimum among ALL siblings (todos + sessions)
    let min_sort: Option<i64> = match (&parent_type, &parent_id) {
        (Some(pt), Some(pid)) => {
            sqlx::query_scalar(
                "SELECT MIN(m) FROM (SELECT MIN(sort_order) AS m FROM todos WHERE parent_type = ?1 AND parent_id = ?2 UNION ALL SELECT MIN(sort_order) FROM sessions WHERE parent_type = ?1 AND parent_id = ?2)"
            )
            .bind(pt)
            .bind(pid)
            .fetch_one(&state.db)
            .await
            .ok()
            .flatten()
        }
        _ => {
            sqlx::query_scalar(
                "SELECT MIN(m) FROM (SELECT MIN(sort_order) AS m FROM todos WHERE parent_type IS NULL UNION ALL SELECT MIN(sort_order) FROM sessions WHERE parent_type IS NULL)"
            )
            .fetch_one(&state.db)
            .await
            .ok()
            .flatten()
        }
    };
    let sort_order = min_sort.unwrap_or(1) - 1;

    let result = sqlx::query_as::<_, Todo>(
        "INSERT INTO todos (text, note, sort_order, created_by_session, parent_type, parent_id) VALUES (?, ?, ?, ?, ?, ?) RETURNING *",
    )
    .bind(&body.text)
    .bind(&note)
    .bind(sort_order)
    .bind(&body.session_id)
    .bind(&parent_type)
    .bind(&parent_id)
    .fetch_one(&state.db)
    .await;

    let _ = state.events_tx.send(AppEvent::TodoUpdated);

    match result {
        Ok(todo) => (StatusCode::CREATED, Json(Some(todo))),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, Json(None)),
    }
}

#[derive(Debug, Deserialize)]
struct UpdateTodo {
    text: Option<String>,
    note: Option<String>,
}

async fn update_todo(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    axum::extract::Form(body): axum::extract::Form<UpdateTodo>,
) -> impl IntoResponse {
    if let Some(text) = &body.text {
        sqlx::query("UPDATE todos SET text = ? WHERE id = ?")
            .bind(text)
            .bind(id)
            .execute(&state.db)
            .await
            .ok();
    }
    if let Some(note) = &body.note {
        sqlx::query("UPDATE todos SET note = ? WHERE id = ?")
            .bind(note)
            .bind(id)
            .execute(&state.db)
            .await
            .ok();
    }
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

// JSON version for the /todo skill API
async fn update_todo_json(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateTodo>,
) -> impl IntoResponse {
    if let Some(text) = &body.text {
        sqlx::query("UPDATE todos SET text = ? WHERE id = ?")
            .bind(text)
            .bind(id)
            .execute(&state.db)
            .await
            .ok();
    }
    if let Some(note) = &body.note {
        sqlx::query("UPDATE todos SET note = ? WHERE id = ?")
            .bind(note)
            .bind(id)
            .execute(&state.db)
            .await
            .ok();
    }
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

async fn delete_todo(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let id_str = id.to_string();

    // Read the todo's parent (grandparent for reparenting children)
    let parent: Option<(Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT parent_type, parent_id FROM todos WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let (gp_type, gp_id) = parent.unwrap_or((None, None));

    // Reparent open child todos to grandparent
    sqlx::query("UPDATE todos SET parent_type = ?, parent_id = ? WHERE parent_type = 'todo' AND parent_id = ? AND status != 'done'")
        .bind(&gp_type)
        .bind(&gp_id)
        .bind(&id_str)
        .execute(&state.db)
        .await
        .ok();
    // Delete done child todos
    sqlx::query("DELETE FROM todos WHERE parent_type = 'todo' AND parent_id = ? AND status = 'done'")
        .bind(&id_str)
        .execute(&state.db)
        .await
        .ok();
    // Reparent child sessions to grandparent
    sqlx::query("UPDATE sessions SET parent_type = ?, parent_id = ? WHERE parent_type = 'todo' AND parent_id = ?")
        .bind(&gp_type)
        .bind(&gp_id)
        .bind(&id_str)
        .execute(&state.db)
        .await
        .ok();

    sqlx::query("DELETE FROM todos WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await
        .ok();
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    StatusCode::OK
}

// --- Card move (reparent + reorder) ---

#[derive(Debug, Deserialize)]
struct MoveCardRequest {
    card_id: String,
    new_parent: Option<String>,
    sibling_order: Vec<String>,
}

async fn move_card(
    State(state): State<AppState>,
    Json(body): Json<MoveCardRequest>,
) -> impl IntoResponse {
    let Some(card_id) = CardId::from_prefixed(&body.card_id) else {
        return StatusCode::BAD_REQUEST;
    };

    let new_parent = body.new_parent.as_deref().and_then(CardId::from_prefixed);

    // Cycle detection: walk up from new_parent to ensure card_id is not an ancestor
    if let Some(ref target) = new_parent {
        let sessions = db::get_sessions(&state.db).await;
        let todos = db::get_todos(&state.db).await;
        let mut current = Some(target.clone());
        while let Some(ref cur) = current {
            if *cur == card_id {
                return StatusCode::BAD_REQUEST; // Would create cycle
            }
            // Find cur's parent
            current = match cur {
                CardId::Session(id) => sessions.iter().find(|s| s.id == *id).and_then(|s| {
                    match (&s.parent_type, &s.parent_id) {
                        (Some(pt), Some(pid)) => CardId::from_db(pt, pid),
                        _ => None,
                    }
                }),
                CardId::Todo(id) => todos.iter().find(|t| t.id == *id).and_then(|t| {
                    match (&t.parent_type, &t.parent_id) {
                        (Some(pt), Some(pid)) => CardId::from_db(pt, pid),
                        _ => None,
                    }
                }),
            };
        }
    }

    // Update parent
    let (parent_type, parent_id) = match &new_parent {
        Some(p) => {
            let (pt, pid) = p.to_db_pair();
            (Some(pt.to_string()), Some(pid))
        }
        None => (None, None),
    };

    match &card_id {
        CardId::Session(id) => {
            sqlx::query("UPDATE sessions SET parent_type = ?, parent_id = ? WHERE id = ?")
                .bind(&parent_type)
                .bind(&parent_id)
                .bind(id)
                .execute(&state.db)
                .await
                .ok();
        }
        CardId::Todo(id) => {
            sqlx::query("UPDATE todos SET parent_type = ?, parent_id = ? WHERE id = ?")
                .bind(&parent_type)
                .bind(&parent_id)
                .bind(id)
                .execute(&state.db)
                .await
                .ok();
        }
    }

    // Update sort_order for all siblings in the target container
    for (i, sibling_prefixed) in body.sibling_order.iter().enumerate() {
        if let Some(sibling_id) = CardId::from_prefixed(sibling_prefixed) {
            match &sibling_id {
                CardId::Session(id) => {
                    sqlx::query("UPDATE sessions SET sort_order = ? WHERE id = ?")
                        .bind(i as i64)
                        .bind(id)
                        .execute(&state.db)
                        .await
                        .ok();
                }
                CardId::Todo(id) => {
                    sqlx::query("UPDATE todos SET sort_order = ? WHERE id = ?")
                        .bind(i as i64)
                        .bind(id)
                        .execute(&state.db)
                        .await
                        .ok();
                }
            }
        }
    }

    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

async fn complete_todo(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("UPDATE todos SET status = 'done', completed_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id)
        .execute(&state.db)
        .await
        .ok();
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

async fn toggle_todo(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> impl IntoResponse {
    let status: Option<String> =
        sqlx::query_scalar("SELECT status FROM todos WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();

    match status.as_deref() {
        Some("done") => {
            sqlx::query("UPDATE todos SET status = 'open', completed_at = NULL WHERE id = ?")
                .bind(id)
                .execute(&state.db)
                .await
                .ok();
        }
        Some(_) => {
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query("UPDATE todos SET status = 'done', completed_at = ? WHERE id = ?")
                .bind(&now)
                .bind(id)
                .execute(&state.db)
                .await
                .ok();
        }
        None => {}
    }
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

async fn clear_todos(State(state): State<AppState>) -> impl IntoResponse {
    sqlx::query("DELETE FROM todos")
        .execute(&state.db)
        .await
        .ok();
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

// --- Settings ---

async fn get_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    let value: Option<String> =
        sqlx::query_scalar("SELECT value FROM settings WHERE key = ?")
            .bind(&key)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    Json(serde_json::json!({ "value": value }))
}

#[derive(Debug, Deserialize)]
struct SetSetting {
    value: String,
}

async fn set_setting(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Json(body): Json<SetSetting>,
) -> impl IntoResponse {
    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = ?",
    )
    .bind(&key)
    .bind(&body.value)
    .bind(&body.value)
    .execute(&state.db)
    .await
    .ok();
    StatusCode::OK
}

// --- Database ---

async fn delete_database(State(state): State<AppState>) -> impl IntoResponse {
    db::delete_all_data(&state.db).await;
    let _ = state.events_tx.send(AppEvent::SessionUpdated);
    let _ = state.events_tx.send(AppEvent::TodoUpdated);
    StatusCode::OK
}

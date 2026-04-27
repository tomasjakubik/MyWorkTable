mod db;
mod models;
mod server;
mod state;
mod time;

use std::net::SocketAddr;

use state::AppState;

#[tokio::main]
async fn main() {
    let pool = db::init_db().await;
    let state = AppState::new(pool);

    // Poll Anthropic usage API in the background.
    tokio::spawn(poll_usage(state.clone()));

    let app = server::router(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:5548")
        .await
        .expect("Failed to bind to port 5548");

    println!("Listening on http://0.0.0.0:5548 (localhost + Docker only)");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .ok();
}

enum FetchOutcome {
    Ok(serde_json::Value),
    Unauthorized,
    Other,
}

async fn fetch_usage(client: &reqwest::Client, token: &str) -> FetchOutcome {
    let resp = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Accept", "application/json")
        .header("Authorization", format!("Bearer {token}"))
        .header("anthropic-beta", "oauth-2025-04-20")
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => match r.json().await {
            Ok(v) => FetchOutcome::Ok(v),
            Err(e) => { eprintln!("[usage] bad json: {e}"); FetchOutcome::Other }
        },
        Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED => {
            eprintln!("[usage] HTTP 401");
            FetchOutcome::Unauthorized
        }
        Ok(r) => { eprintln!("[usage] HTTP {}", r.status()); FetchOutcome::Other }
        Err(e) => { eprintln!("[usage] request failed: {e}"); FetchOutcome::Other }
    }
}

/// Periodically fetch rate-limit utilization from the Anthropic API and store
/// it in the settings table so the dashboard can display it.
async fn poll_usage(state: AppState) {
    let client = reqwest::Client::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));

    loop {
        interval.tick().await;

        let token = match read_access_token() {
            Some(t) => t,
            None => {
                eprintln!("[usage] could not read Claude credentials");
                set_rate_error(&state, "Could not read ~/.claude/.credentials.json").await;
                continue;
            }
        };

        let body = match fetch_usage(&client, &token).await {
            FetchOutcome::Ok(v) => v,
            FetchOutcome::Unauthorized => {
                eprintln!("[usage] refreshing Claude credentials...");
                refresh_credentials().await;

                let new_token = match read_access_token() {
                    Some(t) => t,
                    None => {
                        set_rate_error(&state, "Could not read ~/.claude/.credentials.json").await;
                        continue;
                    }
                };

                match fetch_usage(&client, &new_token).await {
                    FetchOutcome::Ok(v) => v,
                    _ => {
                        set_rate_error(&state, "Authentication failed — re-run `claude` to log in").await;
                        continue;
                    }
                }
            }
            FetchOutcome::Other => continue,
        };

        if let Some(fh) = body.get("five_hour") {
            if let Some(pct) = fh.get("utilization").and_then(|v| v.as_f64()) {
                server::upsert_setting(&state.db, "rate_5h_pct", &format!("{pct:.1}")).await;
            }
            if let Some(resets) = fh.get("resets_at").and_then(|v| v.as_str()) {
                server::upsert_setting(&state.db, "rate_5h_resets", resets).await;
            }
        }
        if let Some(sd) = body.get("seven_day") {
            if let Some(pct) = sd.get("utilization").and_then(|v| v.as_f64()) {
                server::upsert_setting(&state.db, "rate_7d_pct", &format!("{pct:.1}")).await;
            }
            if let Some(resets) = sd.get("resets_at").and_then(|v| v.as_str()) {
                server::upsert_setting(&state.db, "rate_7d_resets", resets).await;
            }
        }
        server::delete_setting(&state.db, "rate_error").await;

        let _ = state.events_tx.send(state::AppEvent::SessionUpdated);
    }
}

/// Spawn `claude` and kill it shortly after — enough time for the CLI to use
/// its long-lived refresh token to mint a new access token and write it back
/// to ~/.claude/.credentials.json.
async fn refresh_credentials() {
    use std::process::Stdio;
    use tokio::process::Command;

    let mut child = match Command::new("claude")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[usage] could not spawn `claude`: {e}");
            return;
        }
    };

    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    let _ = child.start_kill();
    let _ = child.wait().await;
}

/// Wipe the cached rate-limit values and surface a human-readable error so the
/// dashboard renders the red banner instead of stale bars.
async fn set_rate_error(state: &AppState, msg: &str) {
    for k in ["rate_5h_pct", "rate_5h_resets", "rate_7d_pct", "rate_7d_resets"] {
        server::delete_setting(&state.db, k).await;
    }
    server::upsert_setting(&state.db, "rate_error", msg).await;
    let _ = state.events_tx.send(state::AppEvent::SessionUpdated);
}

/// Read the OAuth access token from Claude Code's credentials file.
fn read_access_token() -> Option<String> {
    let home = dirs::home_dir()?;
    let path = home.join(".claude/.credentials.json");
    let data = std::fs::read_to_string(path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&data).ok()?;
    json.get("claudeAiOauth")?
        .get("accessToken")?
        .as_str()
        .map(String::from)
}

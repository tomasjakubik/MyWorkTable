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

    let app = server::router(state);
    let listener = tokio::net::TcpListener::bind("0.0.0.0:5544")
        .await
        .expect("Failed to bind to port 5544");

    println!("Listening on http://0.0.0.0:5544 (localhost + Docker only)");
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .ok();
}

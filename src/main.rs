mod db;
mod models;
mod server;
mod state;
mod time;

use state::AppState;

#[tokio::main]
async fn main() {
    let pool = db::init_db().await;
    let state = AppState::new(pool);

    let router = server::router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:5544")
        .await
        .expect("Failed to bind to port 5544");

    println!("Listening on http://127.0.0.1:5544");
    axum::serve(listener, router).await.ok();
}

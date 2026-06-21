#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "t1ds_signaling_rs=info".into()),
        )
        .init();

    let listen_addr =
        std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let max_rooms = std::env::var("MAX_ROOMS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(t1ds_signaling_rs::DEFAULT_MAX_ROOMS);

    let rooms = t1ds_signaling_rs::new_rooms();
    let app = t1ds_signaling_rs::app(rooms, max_rooms);

    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    tracing::info!(addr = %listen_addr, max_rooms, "signaling server listening");
    axum::serve(listener, app).await.unwrap();
}

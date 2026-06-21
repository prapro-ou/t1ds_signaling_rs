#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "t1ds_signaling_rs=info".into()),
        )
        .init();

    let rooms = t1ds_signaling_rs::new_rooms();
    let app = t1ds_signaling_rs::app(rooms);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    tracing::info!(addr = "0.0.0.0:3000", "signaling server listening");
    axum::serve(listener, app).await.unwrap();
}

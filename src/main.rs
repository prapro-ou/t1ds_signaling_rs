use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use uuid::Uuid;

use room::Rooms;

mod room;

async fn ws_handler(ws: WebSocketUpgrade, State(rooms): State<Rooms>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, rooms))
}

async fn handle_socket(socket: WebSocket, rooms: Rooms) {}

#[tokio::main]
async fn main() {
    let rooms: Rooms = Arc::new(DashMap::new());
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .with_state(rooms);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

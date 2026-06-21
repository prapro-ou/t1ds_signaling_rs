use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
    routing::get,
};
use dashmap::DashMap;
use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::Instrument;

pub mod handler;
pub mod protocol;
pub mod room;

use handler::{handle_host, handle_join, handle_leave, handle_seal, send};
use protocol::SignalMessage;
pub use room::Rooms;

/// Pingを送信する間隔
const PING_INTERVAL: Duration = Duration::from_secs(15);
/// 最後にPongを受信してからこの時間が経過したら切断とみなす
const PONG_TIMEOUT: Duration = Duration::from_secs(45);

/// 空のRoomsを作成する
pub fn new_rooms() -> Rooms {
    Arc::new(DashMap::new())
}

/// シグナリング用のRouterを構築する
pub fn app(rooms: Rooms) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .route("/stats", get(stats_handler))
        .with_state(rooms)
}

#[derive(serde::Serialize)]
struct Stats {
    room_count: usize,
    peer_count: usize,
}

/// 現在のルーム数と接続中の全ピア数を返す
async fn stats_handler(State(rooms): State<Rooms>) -> Json<Stats> {
    let room_count = rooms.len();
    let peer_count = rooms.iter().map(|room| room.clients.len()).sum();
    Json(Stats {
        room_count,
        peer_count,
    })
}

///WebScoket通信にアップグレート
async fn ws_handler(ws: WebSocketUpgrade, State(rooms): State<Rooms>) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(socket, rooms))
}

///WebSocketの処理
async fn handle_socket(socket: WebSocket, rooms: Rooms) {
    let conn_id = uuid::Uuid::new_v4();
    let span = tracing::info_span!("connection", %conn_id);
    handle_socket_inner(socket, rooms).instrument(span).await;
}

async fn handle_socket_inner(socket: WebSocket, rooms: Rooms) {
    tracing::info!("client connected");

    //tx -> rx -> ws_tx -> ws_rx

    // wsを送受信で分離
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    //送信タスクを起動
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_tx.send(msg).await.is_err() {
                break;
            }
        }
    });

    //password,peer_id
    let mut joined: Option<(String, u32)> = None;

    //生存確認用。最後にPongを受信した時刻
    let mut last_pong = Instant::now();
    let mut ping_ticker = interval(PING_INTERVAL);

    //メッセージが届いたら
    'outer: loop {
        let msg = tokio::select! {
            msg = ws_rx.next() => match msg {
                Some(Ok(msg)) => msg,
                _ => break 'outer,
            },
            _ = ping_ticker.tick() => {
                //一定時間Pongが無ければ生存していないとみなし切断
                if last_pong.elapsed() > PONG_TIMEOUT {
                    tracing::warn!("pong timeout, disconnecting");
                    break 'outer;
                }
                if tx.send(Message::Ping(Vec::new().into())).is_err() {
                    break 'outer;
                }
                continue 'outer;
            }
        };

        //Pongを受信したら生存確認の時刻を更新
        if let Message::Pong(_) = msg {
            last_pong = Instant::now();
            continue 'outer;
        }

        //正しい形式か判定
        let Message::Text(text) = msg else { continue };
        let Ok(parsed) = serde_json::from_str::<SignalMessage>(&text) else {
            tracing::warn!(%text, "failed to parse message");
            continue;
        };

        //CMDごとに処理
        match parsed {
            //ホストなら部屋を新規作成する
            SignalMessage::Host {
                password,
                username,
                max_player,
            } => {
                if let Some(j) = handle_host(&rooms, &tx, password, username, max_player) {
                    joined = Some(j);
                }
            }
            //指定したパスワードの部屋に参加する
            SignalMessage::Join { password, username } => {
                if let Some(j) = handle_join(&rooms, &tx, password, username) {
                    joined = Some(j);
                }
            }
            //部屋を退出する
            SignalMessage::Leave {} => {
                handle_leave(&rooms, &mut joined);
            }

            //ホストが部屋を封鎖し、以降の新規参加を拒否する
            SignalMessage::Seal {} => {
                handle_seal(&rooms, &tx, &joined);
            }

            SignalMessage::Offer { target_id, sdp } => {
                relay_to(&rooms, &joined, target_id, |my_id| SignalMessage::Offer {
                    target_id: my_id,
                    sdp,
                });
            }
            SignalMessage::Answer { target_id, sdp } => {
                relay_to(&rooms, &joined, target_id, |my_id| SignalMessage::Answer {
                    target_id: my_id,
                    sdp,
                });
            }

            //通信手段を通知する
            SignalMessage::IceCandidate {
                target_id,
                media,
                index,
                name,
            } => {
                relay_to(&rooms, &joined, target_id, |my_id| {
                    SignalMessage::IceCandidate {
                        target_id: my_id,
                        media,
                        index,
                        name,
                    }
                });
            }
            //サーバー -> クライアントのみのCMDなので無視
            SignalMessage::Id { .. }
            | SignalMessage::HostInfo { .. }
            | SignalMessage::PeerConnect { .. }
            | SignalMessage::PeerDisconnect { .. }
            | SignalMessage::Error { .. } => {}
        }
    }

    //loopから抜ける=wsの接続が切れる
    //退出処理を行う
    handle_leave(&rooms, &mut joined);
    tracing::info!("client disconnected");
}

fn relay_to(
    rooms: &Rooms,
    joined: &Option<(String, u32)>,
    target_id: u32,
    build: impl FnOnce(u32) -> SignalMessage,
) {
    //どこにも所属していないならreturn
    let Some((password, my_id)) = joined else {
        return;
    };
    //自分が入っている部屋が存在しないならreturn
    let Some(room) = rooms.get(password) else {
        return;
    };
    //自分の部屋の人に対してメッセージを送る
    if let Some(target) = room.clients.iter().find(|c| c.id == target_id) {
        send(&target.tx, &build(*my_id));
    }
}

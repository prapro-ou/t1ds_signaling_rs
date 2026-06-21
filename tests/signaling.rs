use std::net::SocketAddr;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

/// テスト用にサーバーをランダムポートで起動し、接続先アドレスを返す。
async fn spawn_server() -> SocketAddr {
    let rooms = t1ds_signaling_rs::new_rooms();
    let app = t1ds_signaling_rs::app(rooms, t1ds_signaling_rs::DEFAULT_MAX_ROOMS);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

async fn connect(addr: SocketAddr) -> WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let url = format!("ws://{addr}/ws");
    let (ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    ws
}

async fn send_json<S>(ws: &mut WebSocketStream<S>, value: Value)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}

/// Ping/Pongは無視して次のテキストメッセージをJSONとして受け取る。
async fn recv_json<S>(ws: &mut WebSocketStream<S>) -> Value
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    loop {
        match ws.next().await.unwrap().unwrap() {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(_) | Message::Pong(_) => continue,
            other => panic!("unexpected message: {other:?}"),
        }
    }
}

/// 生のHTTP GETを送り、ステータスコードとJSONボディを返す。
async fn http_get(addr: SocketAddr, path: &str) -> (u16, Value) {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();

    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let response = String::from_utf8(response).unwrap();

    let (header, body) = response.split_once("\r\n\r\n").unwrap();
    let status = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    (status, serde_json::from_str(body).unwrap())
}

#[tokio::test]
async fn host_join_offer_answer_ice_leave() {
    let addr = spawn_server().await;
    let mut host = connect(addr).await;
    let mut guest = connect(addr).await;

    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":2}),
    )
    .await;
    assert_eq!(recv_json(&mut host).await, json!({"cmd":"Id","id":1}));

    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"test","username":"bob"}),
    )
    .await;
    assert_eq!(recv_json(&mut guest).await, json!({"cmd":"Id","id":2}));
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"HostInfo","username":"alice"})
    );
    assert_eq!(
        recv_json(&mut host).await,
        json!({"cmd":"PeerConnect","id":2})
    );

    // host(1) -> guest(2)
    send_json(
        &mut host,
        json!({"cmd":"Offer","target_id":2,"sdp":"offer-sdp"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"Offer","target_id":1,"sdp":"offer-sdp"})
    );

    // guest(2) -> host(1)
    send_json(
        &mut guest,
        json!({"cmd":"Answer","target_id":1,"sdp":"answer-sdp"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut host).await,
        json!({"cmd":"Answer","target_id":2,"sdp":"answer-sdp"})
    );

    send_json(
        &mut host,
        json!({"cmd":"IceCandidate","target_id":2,"media":"audio","index":0,"name":"candidate"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"IceCandidate","target_id":1,"media":"audio","index":0,"name":"candidate"})
    );

    send_json(&mut guest, json!({"cmd":"Leave"})).await;
    assert_eq!(
        recv_json(&mut host).await,
        json!({"cmd":"PeerDisconnect","id":2})
    );
}

#[tokio::test]
async fn join_with_wrong_password_returns_error() {
    let addr = spawn_server().await;
    let mut guest = connect(addr).await;

    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"no-such-room","username":"bob"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"Error","message":"room not found"})
    );
}

#[tokio::test]
async fn host_leaving_closes_room_for_guest() {
    let addr = spawn_server().await;
    let mut host = connect(addr).await;
    let mut guest = connect(addr).await;

    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":2}),
    )
    .await;
    recv_json(&mut host).await; // Id

    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"test","username":"bob"}),
    )
    .await;
    recv_json(&mut guest).await; // Id
    recv_json(&mut guest).await; // HostInfo
    recv_json(&mut host).await; // PeerConnect

    send_json(&mut host, json!({"cmd":"Leave"})).await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"PeerDisconnect","id":1})
    );
    // ホスト退出後はサーバーから接続が閉じられる
    assert!(matches!(
        guest.next().await.unwrap().unwrap(),
        Message::Close(_)
    ));
}

#[tokio::test]
async fn seal_rejects_new_joins_but_not_existing_peers() {
    let addr = spawn_server().await;
    let mut host = connect(addr).await;
    let mut guest = connect(addr).await;

    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":4}),
    )
    .await;
    recv_json(&mut host).await; // Id

    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"test","username":"bob"}),
    )
    .await;
    recv_json(&mut guest).await; // Id
    recv_json(&mut guest).await; // HostInfo
    recv_json(&mut host).await; // PeerConnect

    send_json(&mut host, json!({"cmd":"Seal"})).await;

    let mut late_guest = connect(addr).await;
    send_json(
        &mut late_guest,
        json!({"cmd":"Join","password":"test","username":"carol"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut late_guest).await,
        json!({"cmd":"Error","message":"room is sealed"})
    );

    // 封鎖前から入っていたピア同士の通信は影響を受けない
    send_json(
        &mut host,
        json!({"cmd":"Offer","target_id":2,"sdp":"offer-sdp"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"Offer","target_id":1,"sdp":"offer-sdp"})
    );
}

#[tokio::test]
async fn seal_by_non_host_returns_error() {
    let addr = spawn_server().await;
    let mut host = connect(addr).await;
    let mut guest = connect(addr).await;

    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":4}),
    )
    .await;
    recv_json(&mut host).await; // Id

    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"test","username":"bob"}),
    )
    .await;
    recv_json(&mut guest).await; // Id
    recv_json(&mut guest).await; // HostInfo
    recv_json(&mut host).await; // PeerConnect

    send_json(&mut guest, json!({"cmd":"Seal"})).await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"Error","message":"only host can seal the room"})
    );

    // 封鎖されていないので新規参加は通常通り成功する
    let mut late_guest = connect(addr).await;
    send_json(
        &mut late_guest,
        json!({"cmd":"Join","password":"test","username":"carol"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut late_guest).await,
        json!({"cmd":"Id","id":3})
    );
}

#[tokio::test]
async fn host_with_duplicate_password_returns_error() {
    let addr = spawn_server().await;
    let mut host = connect(addr).await;
    let mut other_host = connect(addr).await;

    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":4}),
    )
    .await;
    assert_eq!(recv_json(&mut host).await, json!({"cmd":"Id","id":1}));

    send_json(
        &mut other_host,
        json!({"cmd":"Host","password":"test","username":"dave","max_player":4}),
    )
    .await;
    assert_eq!(
        recv_json(&mut other_host).await,
        json!({"cmd":"Error","message":"password already in use"})
    );
}

#[tokio::test]
async fn join_when_room_full_returns_error() {
    let addr = spawn_server().await;
    let mut host = connect(addr).await;
    let mut guest = connect(addr).await;

    // max_player=1なのでホスト自身で既に満員
    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":1}),
    )
    .await;
    recv_json(&mut host).await; // Id

    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"test","username":"bob"}),
    )
    .await;
    assert_eq!(
        recv_json(&mut guest).await,
        json!({"cmd":"Error","message":"room is full"})
    );
}

#[tokio::test]
async fn room_limit_reached_returns_error() {
    let addr = spawn_server().await;

    // MAX_ROOMS(10)まで部屋を埋める
    let mut hosts = Vec::new();
    for i in 0..10 {
        let mut host = connect(addr).await;
        send_json(
            &mut host,
            json!({"cmd":"Host","password":format!("room-{i}"),"username":"alice","max_player":2}),
        )
        .await;
        assert_eq!(recv_json(&mut host).await, json!({"cmd":"Id","id":1}));
        hosts.push(host);
    }

    let mut one_more = connect(addr).await;
    send_json(
        &mut one_more,
        json!({"cmd":"Host","password":"one-too-many","username":"bob","max_player":2}),
    )
    .await;
    assert_eq!(
        recv_json(&mut one_more).await,
        json!({"cmd":"Error","message":"room limit reached"})
    );
}

#[tokio::test]
async fn invalid_json_and_unknown_cmd_are_ignored() {
    let addr = spawn_server().await;
    let mut client = connect(addr).await;

    // 不正なJSON
    client
        .send(Message::Text("not json".into()))
        .await
        .unwrap();
    // 未知のcmd
    client
        .send(Message::Text(
            json!({"cmd":"Unknown","foo":"bar"}).to_string().into(),
        ))
        .await
        .unwrap();

    // 接続は切れておらず、その後の正常なコマンドは処理される
    send_json(
        &mut client,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":2}),
    )
    .await;
    assert_eq!(recv_json(&mut client).await, json!({"cmd":"Id","id":1}));
}

#[tokio::test]
async fn stats_reports_room_and_peer_counts() {
    let addr = spawn_server().await;

    let (status, body) = http_get(addr, "/stats").await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({"room_count": 0, "peer_count": 0}));

    let mut host = connect(addr).await;
    send_json(
        &mut host,
        json!({"cmd":"Host","password":"test","username":"alice","max_player":2}),
    )
    .await;
    recv_json(&mut host).await; // Id

    let mut guest = connect(addr).await;
    send_json(
        &mut guest,
        json!({"cmd":"Join","password":"test","username":"bob"}),
    )
    .await;
    recv_json(&mut guest).await; // Id
    recv_json(&mut guest).await; // HostInfo
    recv_json(&mut host).await; // PeerConnect

    let (status, body) = http_get(addr, "/stats").await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({"room_count": 1, "peer_count": 2}));

    send_json(&mut guest, json!({"cmd":"Leave"})).await;
    recv_json(&mut host).await; // PeerDisconnect

    let (status, body) = http_get(addr, "/stats").await;
    assert_eq!(status, 200);
    assert_eq!(body, json!({"room_count": 1, "peer_count": 1}));
}

//! `/stats` の `turn_allocations` が null 以外になるケースを、
//! coturn の Prometheus エンドポイントを模したモックサーバーで検証する。
//!
//! `app()` は `TURN_METRICS_URL` を env から読む。env はプロセス全体で共有される
//! ため、他テストと干渉しないよう env を差すテストはこのファイルに1つだけ置く
//! (統合テストはファイルごとに別プロセスで動く)。

use std::net::SocketAddr;

use axum::{Router, routing::get};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// coturn の Prometheus 出力を模したモックサーバーを起動する。
async fn spawn_metrics_server(body: &'static str) -> SocketAddr {
    let app = Router::new().route("/metrics", get(move || async move { body }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

/// シグナリングサーバーをランダムポートで起動する。
async fn spawn_signaling() -> SocketAddr {
    let rooms = t1ds_signaling_rs::new_rooms();
    let app = t1ds_signaling_rs::app(rooms, t1ds_signaling_rs::DEFAULT_MAX_ROOMS);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
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

/// TURN_METRICS_URL が設定され、coturn がアロケーション数を返すとき、
/// /stats の turn_allocations にその値が載る。
#[tokio::test]
async fn stats_reports_turn_allocations_from_metrics() {
    // アロケーション数として適当な固定値を返すモック
    let metrics = spawn_metrics_server("turn_total_allocations 42\n").await;

    // app() が env を読むので、シグナリング起動より前に設定する。
    // このファイルには env を触るテストがこれ1つだけなので競合しない。
    // SAFETY: 他スレッドが env を同時に読み書きしていない(単一テスト)。
    unsafe {
        std::env::set_var("TURN_METRICS_URL", format!("http://{metrics}/metrics"));
    }

    let addr = spawn_signaling().await;

    let (status, body) = http_get(addr, "/stats").await;
    assert_eq!(status, 200);
    assert_eq!(body["turn_allocations"], 42);
    assert_eq!(body["room_count"], 0);
    assert_eq!(body["peer_count"], 0);
}

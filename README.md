# t1ds_signaling_rs

WebRTC(Godotの `WebRTCMultiplayerPeer` 想定)向けのシグナリングサーバー。
パスワードで部屋(Room)を作り、ホストと複数のクライアント間でSDP/ICE candidateを中継する。

## 特徴

- WebSocket(`/ws`)1本でホスト作成・参加・SDP/ICE中継・退出を扱う
- パスワード制の部屋管理(同時に存在できる部屋数は上限あり)
- ホストのpeer idは常に`1`固定(`WebRTCMultiplayerPeer`の規約に準拠)
- 部屋封鎖(`Seal`)で以降の新規参加を拒否
- Ping/Pong(15秒間隔送信、45秒無応答で切断)による死活監視
- `/stats`で現在の部屋数・接続中ピア数を取得可能
- `username`は32文字、`password`は64文字までの上限あり(超えると`Error`を返す)

## 起動方法

```bash
cargo run
```

### 環境変数

| 変数 | 説明 | デフォルト |
| --- | --- | --- |
| `LISTEN_ADDR` | 待ち受けアドレス | `0.0.0.0:3000` |
| `MAX_ROOMS` | 同時に存在できる部屋数の上限 | `10` |
| `RUST_LOG` | ログレベル(`tracing-subscriber`の`EnvFilter`) | `t1ds_signaling_rs=info` |

## エンドポイント

- `GET /ws` — シグナリング用WebSocket
- `GET /stats` — `{ "room_count": number, "peer_count": number }` を返す

## プロトコル

`/ws`上でJSONメッセージをやり取りする。各メッセージは`cmd`フィールドで種別を判別する(`src/protocol.rs`)。

### クライアント → サーバー

| cmd | フィールド | 説明 |
| --- | --- | --- |
| `Host` | `password`, `username`, `max_player` | 新しい部屋を作成しホストになる |
| `Join` | `password`, `username` | 指定したパスワードの部屋に参加する |
| `Leave` | - | 部屋から退出する |
| `Seal` | - | 部屋を封鎖し、以降の新規参加を拒否する(ホストのみ) |
| `Offer` | `target_id`, `sdp` | 指定した相手にSDP Offerを中継する |
| `Answer` | `target_id`, `sdp` | 指定した相手にSDP Answerを中継する |
| `IceCandidate` | `target_id`, `media`, `index`, `name` | 指定した相手にICE candidateを中継する |

### サーバー → クライアント

| cmd | フィールド | 説明 |
| --- | --- | --- |
| `Id` | `id` | 自分のpeer idを通知する |
| `HostInfo` | `username` | ホストのユーザー名を通知する(参加時) |
| `PeerConnect` | `id`, `username` | 新規ピアの参加を通知する |
| `PeerDisconnect` | `id`, `username` | ピアの退出を通知する |
| `Error` | `message` | 失敗(部屋が見つからない、満員、封鎖済みなど)を通知する |

ホストが退出した場合はセッションを継続できないため、部屋の全員に`PeerDisconnect`を送ったうえで接続を切断し、部屋自体も破棄する。

## テスト

```bash
cargo test
```

ログを見たい場合は`RUST_LOG`を指定する。

```bash
RUST_LOG=t1ds_signaling_rs=debug cargo test -- --nocapture
```

## デプロイ

`Dockerfile`は`x86_64-unknown-linux-musl`向けにビルドした静的バイナリを`FROM scratch`イメージにコピーする想定(`.cargo/config.toml`参照)。

```bash
cargo build --release --target x86_64-unknown-linux-musl
cp target/x86_64-unknown-linux-musl/release/t1ds_signaling_rs .
docker compose up -d --build
```

`docker-compose.yml`はCaddyをリバースプロキシとして`/ws`・`/stats`を公開する構成。

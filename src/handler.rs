use axum::extract::ws::Message;
use tokio::sync::mpsc;

use crate::protocol::SignalMessage;
use crate::room::{self, Client, CreateRoomError, HOST_PEER_ID, Rooms};

pub(crate) fn send(tx: &mpsc::UnboundedSender<Message>, msg: &SignalMessage) {
    if let Ok(text) = serde_json::to_string(msg) {
        let _ = tx.send(Message::Text(text.into()));
    }
}

/// ホスト失敗の理由をクライアントに通知する。
fn send_host_error(tx: &mpsc::UnboundedSender<Message>, password: &str, e: room::CreateRoomError) {
    let message = match e {
        CreateRoomError::PasswordAlreadyUsed => "password already in use",
        CreateRoomError::RoomLimitReached => "room limit reached",
    };
    tracing::warn!(%password, message, "host failed");
    send(
        tx,
        &SignalMessage::Error {
            message: message.to_string(),
        },
    );
}

/// パスワード・ユーザー名が文字数の上限を超えていないか調べる。
fn validate_credentials(password: &str, username: &str) -> Result<(), &'static str> {
    if password.chars().count() > room::MAX_PASSWORD_LEN {
        return Err("password too long");
    }
    if username.chars().count() > room::MAX_USERNAME_LEN {
        return Err("username too long");
    }
    Ok(())
}

/// ホスト要求を処理する。成功したら (password, peer_id) を返す。
pub(crate) fn handle_host(
    rooms: &Rooms,
    tx: &mpsc::UnboundedSender<Message>,
    password: String,
    username: String,
    max_player: u8,
    max_rooms: usize,
) -> Option<(String, u32)> {
    if let Err(message) = validate_credentials(&password, &username) {
        tracing::warn!(%password, message, "host failed");
        send(
            tx,
            &SignalMessage::Error {
                message: message.to_string(),
            },
        );
        return None;
    }
    let client = Client {
        id: HOST_PEER_ID,
        username,
        tx: tx.clone(),
    };
    //パスワードが既に使われていないかを調べ、使われてないなら部屋を作る
    match room::create_room(rooms, password.clone(), client, max_player, max_rooms) {
        Ok(()) => {
            tracing::info!(%password, max_player, "room created");
            send(tx, &SignalMessage::Id { id: HOST_PEER_ID });
            //(password, peer_id)を返す
            Some((password, HOST_PEER_ID))
        }
        Err(e) => {
            send_host_error(tx, &password, e);
            None
        }
    }
}

/// 部屋を封鎖し、新規参加を拒否する。ホストのみ実行できる。
pub(crate) fn handle_seal(
    rooms: &Rooms,
    tx: &mpsc::UnboundedSender<Message>,
    joined: &Option<(String, u32)>,
) {
    //どこにも所属していないならreturn
    let Some((password, my_id)) = joined else {
        return;
    };
    //自分が入っている部屋が存在しないならreturn
    let Some(mut room) = rooms.get_mut(password) else {
        return;
    };
    //ホスト以外は封鎖できない
    if !room.is_host(*my_id) {
        tracing::warn!(%password, peer_id = my_id, "non-host tried to seal room");
        send(
            tx,
            &SignalMessage::Error {
                message: "only host can seal the room".to_string(),
            },
        );
        return;
    }
    tracing::info!(%password, "room sealed");
    room.is_sealed = true;
}

/// 部屋から退出する。`joined` が部屋に入っていなければ何もしない。
pub(crate) fn handle_leave(rooms: &Rooms, joined: &mut Option<(String, u32)>) {
    let Some((password, my_id)) = joined.take() else {
        return;
    };
    let Some(mut room) = rooms.get_mut(&password) else {
        return;
    };
    let username = room
        .clients
        .iter()
        .find(|c| c.id == my_id)
        .map(|c| c.username.clone())
        .unwrap_or_default();
    let was_host = room.remove_client(my_id);
    tracing::info!(%password, peer_id = my_id, was_host, "peer left room");
    let peer_disconnect_msg = SignalMessage::PeerDisconnect {
        id: my_id,
        username,
    };
    for c in &room.clients {
        send(&c.tx, &peer_disconnect_msg);
        //ホストが抜けた場合、セッションは継続できないので残りの接続も明示的に切る
        if was_host {
            let _ = c.tx.send(Message::Close(None));
        }
    }
    //ホストが抜けた、または誰も残っていないなら部屋を破棄する
    if was_host || room.clients.is_empty() {
        drop(room);
        rooms.remove(&password);
        tracing::info!(%password, "room removed");
    }
}

/// 参加失敗の理由をクライアントに通知する。
fn send_join_error(tx: &mpsc::UnboundedSender<Message>, password: &str, e: room::JoinError) {
    let message = match e {
        room::JoinError::RoomNotFound => "room not found",
        room::JoinError::RoomFull => "room is full",
        room::JoinError::RoomSealed => "room is sealed",
    };
    tracing::warn!(%password, message, "join failed");
    send(
        tx,
        &SignalMessage::Error {
            message: message.to_string(),
        },
    );
}

/// 参加要求を処理する。成功したら (password, peer_id) を返す。
/// 失敗したらNoneを返す
pub(crate) fn handle_join(
    rooms: &Rooms,
    tx: &mpsc::UnboundedSender<Message>,
    password: String,
    username: String,
) -> Option<(String, u32)> {
    if let Err(message) = validate_credentials(&password, &username) {
        tracing::warn!(%password, message, "join failed");
        send(
            tx,
            &SignalMessage::Error {
                message: message.to_string(),
            },
        );
        return None;
    }
    //部屋が見つからなかったらエラーを返す
    let Some(mut room) = rooms.get_mut(&password) else {
        send_join_error(tx, &password, room::JoinError::RoomNotFound);
        return None;
    };
    let new_id = room.next_peer_id();
    let client = Client {
        id: new_id,
        username: username.clone(),
        tx: tx.clone(),
    };

    //参加申請
    if let Err(e) = room.add_client(client) {
        send_join_error(tx, &password, e);
        return None;
    }

    tracing::info!(%password, peer_id = new_id, "peer joined room");
    //全員に新規参加者を通知
    let peer_connect_msg = SignalMessage::PeerConnect {
        id: new_id,
        username,
    };
    for c in &room.clients {
        if c.id != new_id {
            send(&c.tx, &peer_connect_msg);
        }
    }
    //自分のidをクライアントに
    send(tx, &SignalMessage::Id { id: new_id });
    //ホストの名前をクライアントに
    if let Some(username) = room.host_username() {
        send(
            tx,
            &SignalMessage::HostInfo {
                username: username.to_string(),
            },
        );
    }
    //既にいる他のゲストの情報を新規参加者に通知（ホストはHostInfoで通知済み）
    for c in &room.clients {
        if c.id != new_id && c.id != room.host_id {
            send(
                tx,
                &SignalMessage::PeerConnect {
                    id: c.id,
                    username: c.username.clone(),
                },
            );
        }
    }
    Some((password, new_id))
}

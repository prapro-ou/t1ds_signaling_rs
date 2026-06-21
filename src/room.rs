use axum::extract::ws::Message;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

pub enum JoinError {
    RoomNotFound,
    RoomFull,
    RoomSealed,
}

pub enum CreateRoomError {
    PasswordAlreadyUsed,
    RoomLimitReached,
}

/// WebRTCMultiplayerPeerの規約上、ホストのpeer idは常に1。
pub const HOST_PEER_ID: u32 = 1;

/// 同時に存在できる部屋数の上限のデフォルト値。
pub const DEFAULT_MAX_ROOMS: usize = 10;

/// ユーザー名の最大文字数。
pub const MAX_USERNAME_LEN: usize = 32;

/// パスワードの最大文字数。
pub const MAX_PASSWORD_LEN: usize = 64;

pub struct Client {
    pub id: u32,
    pub username: String,
    pub tx: UnboundedSender<Message>,
}

pub struct Room {
    pub max_player: u8,
    pub host_id: u32,
    pub clients: Vec<Client>,
    pub is_sealed: bool,
}

impl Room {
    /// 与えられたidがホストか調べる
    pub fn is_host(&self, client_id: u32) -> bool {
        self.host_id == client_id
    }

    /// ホストのユーザー名を取得
    pub fn host_username(&self) -> Option<&str> {
        self.clients
            .iter()
            .find(|c| c.id == self.host_id)
            .map(|c| c.username.as_str())
    }

    pub fn add_client(&mut self, client: Client) -> Result<(), JoinError> {
        if self.is_sealed {
            return Err(JoinError::RoomSealed);
        }

        if self.clients.len() >= self.max_player as usize {
            return Err(JoinError::RoomFull);
        }
        self.clients.push(client);
        Ok(())
    }

    /// クライアントを削除する。
    /// 削除したクライアントがホストだった場合は `true` を返す。
    pub fn remove_client(&mut self, client_id: u32) -> bool {
        let was_host = self.is_host(client_id);
        self.clients.retain(|c| c.id != client_id);
        was_host
    }

    /// まだ使われていないpeer idを探す。
    pub fn next_peer_id(&self) -> u32 {
        let mut id = HOST_PEER_ID + 1;
        while self.clients.iter().any(|c| c.id == id) {
            id += 1;
        }
        id
    }
}

/// password -> Room
pub type Rooms = Arc<DashMap<String, Room>>;

/// 指定したpasswordで新しい部屋を作る。同じpasswordの部屋が既にあればエラー。
pub fn create_room(
    rooms: &Rooms,
    password: String,
    host: Client,
    max_player: u8,
    max_rooms: usize,
) -> Result<(), CreateRoomError> {
    if rooms.len() >= max_rooms {
        return Err(CreateRoomError::RoomLimitReached);
    }
    match rooms.entry(password) {
        Entry::Occupied(_) => Err(CreateRoomError::PasswordAlreadyUsed),
        Entry::Vacant(entry) => {
            entry.insert(Room {
                max_player,
                host_id: host.id,
                clients: vec![host],
                is_sealed: false,
            });
            Ok(())
        }
    }
}

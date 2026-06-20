use axum::extract::ws::Message;
use dashmap::DashMap;
use dashmap::mapref::entry::Entry;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

pub enum JoinError {
    RoomFull,
}

pub enum CreateRoomError {
    PasswordAlreadyUsed,
}

pub struct Client {
    pub id: String,
    pub username: String,
    pub tx: UnboundedSender<Message>,
}

pub struct Room {
    pub max_player: u8,
    pub host_id: String,
    pub clients: Vec<Client>,
}

impl Room {
    /// 与えられたidがホストか調べる
    pub fn is_host(&self, client_id: &str) -> bool {
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
        if self.clients.len() >= self.max_player as usize {
            return Err(JoinError::RoomFull);
        }
        self.clients.push(client);
        Ok(())
    }

    /// クライアントを削除する。
    /// 削除したクライアントがホストだった場合は `true` を返す。
    pub fn remove_client(&mut self, client_id: &str) -> bool {
        let was_host = self.is_host(client_id);
        self.clients.retain(|c| c.id != client_id);
        was_host
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
) -> Result<(), CreateRoomError> {
    match rooms.entry(password) {
        Entry::Occupied(_) => Err(CreateRoomError::PasswordAlreadyUsed),
        Entry::Vacant(entry) => {
            entry.insert(Room {
                max_player,
                host_id: host.id.clone(),
                clients: vec![host],
            });
            Ok(())
        }
    }
}

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub credential: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "cmd")]
pub enum SignalMessage {
    Host {
        password: String,
        username: String,
        max_player: u8,
    },
    Join {
        password: String,
        username: String,
    },
    IceServers {
        ice_servers: Vec<IceServer>,
    },
    Leave {},
    Offer {
        target_id: u32,
        sdp: String,
    },
    Answer {
        target_id: u32,
        sdp: String,
    },
    IceCandidate {
        target_id: u32,
        media: String,
        index: i32,
        name: String,
    },
    Seal {},
    Id {
        id: u32,
    },
    HostInfo {
        username: String,
    },
    PeerConnect {
        id: u32,
        username: String,
    },
    PeerDisconnect {
        id: u32,
        username: String,
    },
    Error {
        message: String,
    },
}

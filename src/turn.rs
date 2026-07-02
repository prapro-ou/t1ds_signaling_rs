use std::time::{SystemTime, UNIX_EPOCH};

use base64::{Engine, engine::general_purpose};
use hmac::{Hmac, KeyInit, Mac};
use sha1::Sha1;

use crate::protocol::IceServer;

/// msgを鍵でハッシュ化してbase64にして返す
pub fn base64_hamc_sha1(secret: &str, msg: &str) -> String {
    //鍵で初期化
    let mut mac =
        Hmac::<Sha1>::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any size");

    //msgをハッシュ化
    mac.update(msg.as_bytes());

    //定数時間で比較する状態にラップして取り出す
    let code = mac.finalize().into_bytes();

    //base64に
    general_purpose::STANDARD.encode(code)
}

/// env から構築する TURN 配布設定。
#[derive(Clone)]
pub struct TurnConfig {
    secret: String,
    host: String,
    ttl_secs: u64,
}

impl TurnConfig {
    /// TURN_SECRET と TURN_HOST が両方あるときだけ Some。
    /// 片方でも無ければ None＝TURN配布を無効化。
    pub fn from_env() -> Option<Self> {
        let secret = std::env::var("TURN_SECRET")
            .ok()
            .filter(|s| !s.is_empty())?;
        let host = std::env::var("TURN_HOST").ok().filter(|s| !s.is_empty())?;
        let ttl_secs = std::env::var("TURN_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(21_600);
        Some(Self {
            secret,
            host,
            ttl_secs,
        })
    }
    /// peer ごとの使い捨てクレデンシャルを載せた ICE サーバー一覧を作る。
    pub fn ice_servers(&self, peer_id: u32) -> Vec<IceServer> {
        //現在のUNIX秒 + TTL を有効期限にする
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before UNIX epoch")
            .as_secs();
        let expiry = now + self.ttl_secs;

        let username = format!("{expiry}:{peer_id}");
        let credential = base64_hamc_sha1(&self.secret, &username);

        vec![
            IceServer {
                urls: vec![format!("stun:{}:3478", self.host)],
                username: None,
                credential: None,
            },
            IceServer {
                urls: vec![
                    format!("turn:{}:3478?transport=udp", self.host),
                    format!("turn:{}:3478?transport=tcp", self.host),
                    format!("turns:{}:5349?transport=tcp", self.host),
                ],
                username: Some(username),
                credential: Some(credential),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 2202 の HMAC-SHA1 テストベクタで、鍵付きハッシュ→base64が仕様どおりか固定する。
    /// (coturn 側の期待値とアルゴリズムが一致していることの担保)
    #[test]
    fn base64_hmac_sha1_matches_known_vector() {
        let got = base64_hamc_sha1("Jefe", "what do ya want for nothing?");
        assert_eq!(got, "7/zfauXrL6LSdBbV8YTfnCWafHk=");
    }

    /// 同じ入力なら決定的、鍵やメッセージが違えば結果も変わる。
    #[test]
    fn base64_hmac_sha1_is_deterministic_and_input_sensitive() {
        assert_eq!(
            base64_hamc_sha1("secret", "1000:2"),
            base64_hamc_sha1("secret", "1000:2"),
        );
        assert_ne!(
            base64_hamc_sha1("secret", "1000:2"),
            base64_hamc_sha1("secret", "1000:3"),
        );
        assert_ne!(
            base64_hamc_sha1("secret-a", "1000:2"),
            base64_hamc_sha1("secret-b", "1000:2"),
        );
    }

    fn test_config() -> TurnConfig {
        TurnConfig {
            secret: "test-secret".to_string(),
            host: "turn.example.com".to_string(),
            ttl_secs: 3600,
        }
    }

    /// STUN 1件 + TURN 1件を返し、STUNは認証なし・TURNは認証ありになる。
    #[test]
    fn ice_servers_returns_stun_and_turn() {
        let servers = test_config().ice_servers(2);
        assert_eq!(servers.len(), 2);

        let stun = &servers[0];
        assert_eq!(stun.urls, vec!["stun:turn.example.com:3478".to_string()]);
        assert!(stun.username.is_none());
        assert!(stun.credential.is_none());

        let turn = &servers[1];
        let expected_urls: Vec<String> = vec![
            "turn:turn.example.com:3478?transport=udp".to_string(),
            "turn:turn.example.com:3478?transport=tcp".to_string(),
            "turns:turn.example.com:5349?transport=tcp".to_string(),
        ];
        assert_eq!(turn.urls, expected_urls);
        assert!(turn.username.is_some());
        assert!(turn.credential.is_some());
    }

    /// username は `<expiry>:<peer_id>` 形式で、credential はその username を鍵でハッシュした値。
    #[test]
    fn ice_servers_credentials_follow_auth_secret_scheme() {
        let cfg = test_config();
        let peer_id = 7;
        let turn = cfg.ice_servers(peer_id).pop().unwrap();

        let username = turn.username.unwrap();
        let (expiry, id) = username
            .split_once(':')
            .expect("username has <expiry>:<peer_id> form");
        assert_eq!(id.parse::<u32>().unwrap(), peer_id);

        // expiry は実在するUNIX秒(=2023-11以降)であること
        let expiry: u64 = expiry.parse().expect("expiry is a unix timestamp");
        assert!(expiry > 1_700_000_000);

        // credential = base64(HMAC-SHA1(secret, username))
        assert_eq!(
            turn.credential.unwrap(),
            base64_hamc_sha1("test-secret", &username),
        );
    }

    /// peer_id が違えば username(=クレデンシャル)も変わる=ピアごとに使い捨て。
    #[test]
    fn ice_servers_differ_per_peer() {
        let cfg = test_config();
        let a = cfg.ice_servers(2).pop().unwrap().username.unwrap();
        let b = cfg.ice_servers(3).pop().unwrap().username.unwrap();
        assert_ne!(a, b);
    }
}

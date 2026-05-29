use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub content: String,
    pub sender: String,
    pub timestamp: DateTime<Utc>,
    pub is_encrypted: bool,
    pub is_mine: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum P2PMessage {
    #[serde(rename = "key_exchange")]
    KeyExchange {
        public_key: String,
        peer_id: String,
        signature: String,
    },
    #[serde(rename = "key_exchange_ack")]
    KeyExchangeAck {
        public_key: String,
        peer_id: String,
        signature: String,
    },
    #[serde(rename = "session_confirm")]
    SessionConfirm {
        fingerprint: String,
        peer_id: String,
    },
    #[serde(rename = "text")]
    Text {
        nonce: String,
        ciphertext: String,
        seq: u64,
        timestamp: i64,
    },
    #[serde(rename = "typing")]
    Typing {
        is_typing: bool,
    },
    #[serde(rename = "ping")]
    Ping { timestamp: i64 },
    #[serde(rename = "pong")]
    Pong { timestamp: i64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected { is_direct: bool },
}

#[derive(Debug, Clone)]
pub enum NetworkEvent {
    Registered {
        code: String,
        #[allow(dead_code)]
        peer_id: String,
    },
    #[allow(dead_code)]
    ConnectionRequest {
        from_peer: String,
        from_ip: String,
        from_port: u16,
        from_code: String,
    },
    #[allow(dead_code)]
    ConnectionInfo {
        peer_id: String,
        public_ip: String,
        port: u16,
        code: String,
    },
    #[allow(dead_code)]
    P2PMessageReceived {
        message: P2PMessage,
    },
    Connected {
        #[allow(dead_code)]
        peer_id: String,
        is_direct: bool,
    },
    Disconnected,
    Error {
        message: String,
    },
    MessageReceived {
        text: String,
        #[allow(dead_code)]
        seq: u64,
        sender: String,
        timestamp: DateTime<Utc>,
    },
    PeerTyping {
        is_typing: bool,
    },
    PeerFingerprint {
        fingerprint: String,
    },
}

#[derive(Debug, Clone)]
pub enum NetworkCommand {
    ConnectToCode { code: String },
    SendMessage { text: String },
    #[allow(dead_code)]
    SendTyping { is_typing: bool },
    Disconnect,
}

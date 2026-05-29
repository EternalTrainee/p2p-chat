use std::path::PathBuf;

use anyhow::Result;
use chrono::{DateTime, Utc};
use rusqlite::Connection;
use uuid::Uuid;

use crate::types::ChatMessage;

pub struct Storage {
    conn: Connection,
}

impl Storage {
    pub fn new(data_dir: Option<PathBuf>) -> Result<Self> {
        let dir = data_dir.unwrap_or_else(|| {
            let mut d = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
            d.push("p2p-chat");
            d
        });
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("p2p-chat.db");
        let conn = Connection::open(&db_path)?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS identity (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            );
            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                sender TEXT NOT NULL,
                timestamp INTEGER NOT NULL,
                is_encrypted INTEGER NOT NULL DEFAULT 1,
                is_mine INTEGER NOT NULL DEFAULT 0
            );
            CREATE TABLE IF NOT EXISTS peers (
                code TEXT PRIMARY KEY,
                peer_id TEXT,
                fingerprint TEXT,
                last_seen INTEGER
            );
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp);
            ",
        )?;

        Ok(Self { conn })
    }

    pub fn load_identity_seed(&self) -> Result<[u8; 32]> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM identity WHERE key = 'identity_seed'")?;
        let result: Option<Vec<u8>> = stmt
            .query_row([], |row| row.get(0))
            .ok();

        match result {
            Some(bytes) => {
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&bytes);
                Ok(seed)
            }
            None => {
                let mut seed = [0u8; 32];
                use rand::RngCore;
                rand::rngs::OsRng.fill_bytes(&mut seed);
                self.conn.execute(
                    "INSERT OR REPLACE INTO identity (key, value) VALUES ('identity_seed', ?1)",
                    [seed.to_vec()],
                )?;
                Ok(seed)
            }
        }
    }

    pub fn save_message(&self, msg: &ChatMessage) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO messages (id, content, sender, timestamp, is_encrypted, is_mine)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                msg.id,
                msg.content,
                msg.sender,
                msg.timestamp.timestamp(),
                msg.is_encrypted as i32,
                msg.is_mine as i32,
            ],
        )?;
        Ok(())
    }

    pub fn load_messages(&self, limit: i64) -> Result<Vec<ChatMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, content, sender, timestamp, is_encrypted, is_mine
             FROM messages ORDER BY timestamp DESC LIMIT ?1",
        )?;
        let msgs = stmt
            .query_map(rusqlite::params![limit], |row| {
                let ts: i64 = row.get(3)?;
                Ok(ChatMessage {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    sender: row.get(2)?,
                    timestamp: DateTime::from_timestamp(ts, 0).unwrap_or_default(),
                    is_encrypted: row.get::<_, i32>(4)? != 0,
                    is_mine: row.get::<_, i32>(5)? != 0,
                })
            })?
            .filter_map(|r| r.ok())
            .collect::<Vec<_>>();

        Ok(msgs.into_iter().rev().collect())
    }

    #[allow(dead_code)]
    pub fn save_peer(&self, code: &str, peer_id: &str, fingerprint: Option<&str>) -> Result<()> {
        self.conn.execute(
            "INSERT OR REPLACE INTO peers (code, peer_id, fingerprint, last_seen)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                code,
                peer_id,
                fingerprint,
                chrono::Utc::now().timestamp(),
            ],
        )?;
        Ok(())
    }

    pub fn store_message(
        &self,
        content: &str,
        sender: &str,
        is_mine: bool,
    ) -> Result<ChatMessage> {
        let msg = ChatMessage {
            id: Uuid::new_v4().to_string(),
            content: content.to_string(),
            sender: sender.to_string(),
            timestamp: Utc::now(),
            is_encrypted: true,
            is_mine,
        };
        self.save_message(&msg)?;
        Ok(msg)
    }
}

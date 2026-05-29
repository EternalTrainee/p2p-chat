use std::time::Duration;

use anyhow::Result;
use chrono::Utc;
use futures_util::SinkExt;
use log::{error, info, warn};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

use crate::crypto::CryptoEngine;
use crate::types::{NetworkCommand, NetworkEvent, P2PMessage};

#[derive(Serialize, Deserialize, Debug)]
struct WsMessage {
    #[serde(rename = "type")]
    msg_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

async fn p2p_write(stream: &mut TcpStream, msg: &P2PMessage) -> Result<()> {
    let data = serde_json::to_vec(msg)?;
    let len = data.len() as u32;
    stream.write_all(&len.to_be_bytes()).await?;
    stream.write_all(&data).await?;
    stream.flush().await?;
    Ok(())
}

async fn p2p_read(stream: &mut TcpStream) -> Result<P2PMessage> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf).await?;
    Ok(serde_json::from_slice(&buf)?)
}

pub async fn run_network(
    mut cmd_rx: mpsc::UnboundedReceiver<NetworkCommand>,
    event_tx: mpsc::UnboundedSender<NetworkEvent>,
    signaling_url: String,
    p2p_port: u16,
    identity_seed: [u8; 32],
    public_ip: String,
) {
    let mut crypto = CryptoEngine::new(&identity_seed);
    let mut p2p_stream: Option<TcpStream> = None;

    // Bind TCP listener FIRST to know our actual port
    let listener = match TcpListener::bind(format!("0.0.0.0:{}", p2p_port)).await {
        Ok(l) => {
            info!("P2P listening on port {}", l.local_addr().unwrap().port());
            l
        }
        Err(e) => {
            error!("Failed to bind P2P port: {}", e);
            return;
        }
    };
    let actual_port = listener.local_addr().unwrap().port();

    // Connect to signaling server with retry
    let (mut ws_tx, mut ws_rx) = loop {
        match connect_signaling(&signaling_url, actual_port, &event_tx, &public_ip).await {
            Ok((tx, rx, code)) => {
                let _ = event_tx.send(NetworkEvent::Registered {
                    code: code.clone(),
                    peer_id: crypto.public_key_base64(),
                });
                break (tx, rx);
            }
            Err(e) => {
                error!("Failed to connect to signaling (retry 3s): {}", e);
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
        }
    };

    loop {
        let p2p_fut = async {
            match p2p_stream.as_mut() {
                Some(stream) => p2p_read(stream).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            accept = listener.accept() => {
                if let Ok((mut stream, addr)) = accept {
                    if p2p_stream.is_some() {
                        warn!("Already connected, rejecting {}", addr);
                        continue;
                    }
                    info!("Incoming P2P from {}", addr);

                    let result: Result<()> = async {
                        let msg = p2p_read(&mut stream).await?;
                        crypto.process_key_exchange(&msg)?;
                        let ack = crypto.create_key_exchange_ack("");
                        p2p_write(&mut stream, &ack).await?;
                        crypto.derive_shared_key()?;
                        // Read session confirm from initiator
                        let confirm = p2p_read(&mut stream).await?;
                        if let P2PMessage::SessionConfirm { fingerprint, .. } = &confirm {
                            crypto.set_peer_fingerprint(fingerprint);
                        }
                        // Send our session confirm
                        if let Some(our_confirm) = crypto.create_session_confirm() {
                            p2p_write(&mut stream, &our_confirm).await?;
                        }
                        Ok(())
                    }.await;

                    match result {
                        Ok(()) => {
                            if let Some(fp) = crypto.fingerprint() {
                                let _ = event_tx.send(NetworkEvent::PeerFingerprint {
                                    fingerprint: fp.to_string(),
                                });
                            }
                            let _ = event_tx.send(NetworkEvent::Connected {
                                peer_id: crypto.peer_id().unwrap_or("peer").to_string(),
                                is_direct: true,
                            });
                            p2p_stream = Some(stream);
                        }
                        Err(e) => {
                            error!("P2P accept handshake failed: {}", e);
                        }
                    }
                }
            }
            ws_msg = ws_rx.recv() => {
                match ws_msg {
                    Some(text) => {
                        let ws_msg: WsMessage = match serde_json::from_str(&text) {
                            Ok(m) => m,
                            Err(_) => continue,
                        };
                        match ws_msg.msg_type.as_str() {
                            "connection_info" => {
                                let ip = ws_msg.public_ip.unwrap_or_default();
                                let port = ws_msg.port.unwrap_or(0);
                                let addr = format!("{}:{}", ip, port);
                                info!("Connecting to peer at {}", addr);

                                let result: Result<()> = async {
                                    let mut stream = TcpStream::connect(&addr).await?;
                                    let msg = crypto.create_key_exchange_message("");
                                    p2p_write(&mut stream, &msg).await?;
                                    let ack = p2p_read(&mut stream).await?;
                                    crypto.process_key_exchange(&ack)?;
                                    crypto.derive_shared_key()?;
                                    // Send our session confirm
                                    if let Some(confirm) = crypto.create_session_confirm() {
                                        p2p_write(&mut stream, &confirm).await?;
                                    }
                                    // Read peer's session confirm
                                    let confirm = p2p_read(&mut stream).await?;
                                    if let P2PMessage::SessionConfirm { fingerprint, .. } = &confirm {
                                        crypto.set_peer_fingerprint(fingerprint);
                                    }
                                    p2p_stream = Some(stream);
                                    Ok(())
                                }.await;

                                match result {
                                    Ok(()) => {
                                        if let Some(fp) = crypto.fingerprint() {
                                            let _ = event_tx.send(NetworkEvent::PeerFingerprint {
                                                fingerprint: fp.to_string(),
                                            });
                                        }
                                        let _ = event_tx.send(NetworkEvent::Connected {
                                            peer_id: ws_msg.peer_id.unwrap_or_default(),
                                            is_direct: true,
                                        });
                                    }
                                    Err(e) => {
                                        warn!("Direct connection failed: {}", e);
                                        let _ = event_tx.send(NetworkEvent::Error {
                                            message: format!("Connection failed: {}. Try port forwarding.", e),
                                        });
                                    }
                                }
                            }
                            "connection_request" => {
                                let _ = event_tx.send(NetworkEvent::Connected {
                                    peer_id: ws_msg.from_peer.unwrap_or_default(),
                                    is_direct: false,
                                });
                            }
                            "error" => {
                                let _ = event_tx.send(NetworkEvent::Error {
                                    message: ws_msg.message.unwrap_or_default(),
                                });
                            }
                            _ => {}
                        }
                    }
                    None => {
                        info!("Signaling disconnected, reconnecting...");
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        loop {
                            match connect_signaling(
                                &signaling_url,
                                listener.local_addr().unwrap().port(),
                                &event_tx,
                                &public_ip,
                            ).await {
                                Ok((tx, rx, _code)) => {
                                    ws_tx = tx;
                                    ws_rx = rx;
                                    break;
                                }
                                Err(e) => {
                                    error!("Reconnect failed: {}", e);
                                    tokio::time::sleep(Duration::from_secs(3)).await;
                                }
                            }
                        }
                    }
                }
            }
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(NetworkCommand::ConnectToCode { code }) => {
                        let msg = serde_json::json!({"type": "connect", "target_code": code});
                        let _ = ws_tx.send(msg.to_string());
                    }
                    Some(NetworkCommand::SendMessage { text }) => {
                        if let Some(ref mut stream) = p2p_stream {
                            match crypto.encrypt_message(&text) {
                                Ok(encrypted) => {
                                    if let Err(e) = p2p_write(stream, &encrypted).await {
                                        error!("P2P write error: {}", e);
                                        let _ = event_tx.send(NetworkEvent::Error {
                                            message: "Failed to send message".into(),
                                        });
                                    }
                                }
                                Err(e) => error!("Encrypt error: {}", e),
                            }
                        }
                    }
                    Some(NetworkCommand::SendTyping { is_typing }) => {
                        if let Some(ref mut stream) = p2p_stream {
                            let msg = P2PMessage::Typing { is_typing };
                            let _ = p2p_write(stream, &msg).await;
                        }
                    }
                    Some(NetworkCommand::Disconnect) => {
                        p2p_stream = None;
                        crypto.reset_session();
                        let _ = event_tx.send(NetworkEvent::Disconnected);
                    }
                    None => break,
                }
            }
            p2p_msg = p2p_fut => {
                match p2p_msg {
                    Ok(msg) => {
                        match msg {
                            P2PMessage::Text { nonce, ciphertext, seq, timestamp } => {
                                let text_msg = P2PMessage::Text { nonce, ciphertext, seq, timestamp };
                                match crypto.decrypt_message(&text_msg) {
                                    Ok(text) => {
                                        let _ = event_tx.send(NetworkEvent::MessageReceived {
                                            text,
                                            seq,
                                            sender: "peer".to_string(),
                                            timestamp: Utc::now(),
                                        });
                                    }
                                    Err(e) => error!("Decrypt error: {}", e),
                                }
                            }
                            P2PMessage::Typing { is_typing } => {
                                let _ = event_tx.send(NetworkEvent::PeerTyping { is_typing });
                            }
                            P2PMessage::Ping { .. } => {
                                if let Some(ref mut stream) = p2p_stream {
                                    let pong = P2PMessage::Pong {
                                        timestamp: Utc::now().timestamp(),
                                    };
                                    let _ = p2p_write(stream, &pong).await;
                                }
                            }
                            _ => {}
                        }
                    }
                    Err(_) => {
                        info!("P2P connection lost");
                        p2p_stream = None;
                        crypto.reset_session();
                        let _ = event_tx.send(NetworkEvent::Disconnected);
                    }
                }
            }
        }
    }
}

async fn connect_signaling(
    url: &str,
    port: u16,
    event_tx: &mpsc::UnboundedSender<NetworkEvent>,
    public_ip: &str,
) -> Result<(
    mpsc::UnboundedSender<String>,
    mpsc::UnboundedReceiver<String>,
    String,
)> {
    // Ensure URL has a path
    let url = if !url.contains("://") {
        format!("ws://{}/", url)
    } else if url.ends_with('/') || url.contains("?") {
        url.to_string()
    } else {
        format!("{}/", url)
    };
    let (ws_stream, _) = connect_async(&url).await?;
    let (mut writer, reader) = ws_stream.split();

    let mut register = serde_json::json!({"type": "register", "port": port});
    if !public_ip.is_empty() {
        register["public_ip"] = serde_json::json!(public_ip);
    }
    writer.send(Message::Text(register.to_string())).await?;

    // Channel for sending messages TO the WebSocket
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    // Spawn writer task
    tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if writer.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    // Read registration response
    use futures_util::StreamExt;
    let mut reader = reader;
    let mut my_code = String::new();
    if let Some(Ok(Message::Text(text))) = reader.next().await {
        if let Ok(msg) = serde_json::from_str::<WsMessage>(&text) {
            if msg.msg_type == "registered" {
                my_code = msg.code.unwrap_or_default();
                info!("Registered with code: {}", my_code);
                let _ = event_tx.send(NetworkEvent::Registered {
                    code: my_code.clone(),
                    peer_id: msg.peer_id.unwrap_or_default(),
                });
            }
        }
    }

    // Spawn reader task
    let (msg_tx, msg_rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            match reader.next().await {
                Some(Ok(Message::Text(text))) => {
                    let _ = msg_tx.send(text.to_string());
                }
                Some(Ok(Message::Close(_))) | None => break,
                _ => {}
            }
        }
    });

    Ok((out_tx, msg_rx, my_code))
}

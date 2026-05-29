use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, RwLock};
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

type PeerMap = Arc<RwLock<HashMap<String, PeerEntry>>>;
type Tx = mpsc::UnboundedSender<String>;

#[derive(Debug, Clone)]
struct PeerEntry {
    peer_id: String,
    code: String,
    port: u16,
    public_ip: String,
    tx: Tx,
}

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
    from_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    to_peer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    from_code: Option<String>,
}

#[derive(Parser)]
#[command(name = "signaling-server")]
struct Cli {
    #[arg(long, default_value = "0.0.0.0")]
    host: String,
    #[arg(short, long, default_value_t = 8080)]
    port: u16,
}

fn generate_code() -> String {
    let mut rng = rand::thread_rng();
    let part1: String = (0..4)
        .map(|_| {
            let c = rng.gen_range(0..36);
            if c < 10 {
                (b'0' + c) as char
            } else {
                (b'A' + c - 10) as char
            }
        })
        .collect();
    let part2: String = (0..4)
        .map(|_| {
            let c = rng.gen_range(0..36);
            if c < 10 {
                (b'0' + c) as char
            } else {
                (b'A' + c - 10) as char
            }
        })
        .collect();
    format!("{}-{}", part1, part2)
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let cli = Cli::parse();

    let addr = format!("{}:{}", cli.host, cli.port);
    let listener = TcpListener::bind(&addr).await?;
    info!("Signaling server running on {}", addr);

    let peer_map: PeerMap = Arc::new(RwLock::new(HashMap::new()));

    while let Ok((stream, peer_addr)) = listener.accept().await {
        let peer_map = peer_map.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(stream, peer_addr, peer_map).await {
                error!("Connection error from {}: {}", peer_addr, e);
            }
        });
    }

    Ok(())
}

async fn handle_connection(
    raw_stream: tokio::net::TcpStream,
    addr: SocketAddr,
    peer_map: PeerMap,
) -> Result<()> {
    let ws_stream = accept_async(raw_stream).await?;
    info!("New WebSocket connection from {}", addr);

    let (mut ws_sender, mut ws_receiver) = ws_stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();

    // Wait for register message
    let register_msg = match ws_receiver.next().await {
        Some(Ok(Message::Text(text))) => text,
        _ => {
            warn!("Peer {} disconnected before registering", addr);
            return Ok(());
        }
    };

    let register: WsMessage = serde_json::from_str(&register_msg)?;
    if register.msg_type != "register" {
        warn!("Peer {} sent unexpected first message type: {}", addr, register.msg_type);
        return Ok(());
    }

    let peer_id = uuid::Uuid::new_v4().to_string();
    let code = generate_code();
    let port = register.port.unwrap_or(0);
    let public_ip = register
        .public_ip
        .clone()
        .unwrap_or_else(|| addr.ip().to_string());

    let entry = PeerEntry {
        peer_id: peer_id.clone(),
        code: code.clone(),
        port,
        public_ip: public_ip.clone(),
        tx: tx.clone(),
    };

    peer_map.write().await.insert(code.clone(), entry);

    let reg_confirm = serde_json::json!({
        "type": "registered",
        "peer_id": peer_id,
        "code": code,
        "public_ip": public_ip,
    });
    ws_sender
        .send(Message::Text(reg_confirm.to_string()))
        .await?;

    info!("Peer registered: {} -> code={}, port={}", peer_id, code, port);

    let code_clone = code.clone();
    let peer_id_clone = peer_id.clone();

    loop {
        tokio::select! {
            Some(Ok(msg)) = ws_receiver.next() => {
                match msg {
                    Message::Text(text) => {
                        if let Err(e) = process_message(
                            &text,
                            &peer_map,
                            &code_clone,
                            &peer_id_clone,
                            &mut ws_sender,
                        ).await {
                            error!("Error processing message from {}: {}", peer_id_clone, e);
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            Some(msg) = rx.recv() => {
                if ws_sender.send(Message::Text(msg)).await.is_err() {
                    break;
                }
            }
            else => break,
        }
    }

    // Cleanup
    peer_map.write().await.remove(&code_clone);
    info!("Peer disconnected: {} (code={})", peer_id_clone, code_clone);
    Ok(())
}

async fn process_message(
    text: &str,
    peer_map: &PeerMap,
    my_code: &str,
    my_peer_id: &str,
    ws_sender: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>,
        Message,
    >,
) -> Result<()> {
    let msg: WsMessage = serde_json::from_str(text)?;

    match msg.msg_type.as_str() {
        "connect" => {
            let target_code = match &msg.target_code {
                Some(c) => c.clone(),
                None => {
                    let err = serde_json::json!({
                        "type": "error",
                        "message": "Missing target_code"
                    });
                    ws_sender.send(Message::Text(err.to_string())).await?;
                    return Ok(());
                }
            };

            let map = peer_map.read().await;
            if let Some(target) = map.get(&target_code) {
                let my_info = map.get(my_code);
                let my_ip = my_info.map(|i| i.public_ip.clone()).unwrap_or_default();
                let my_port = my_info.map(|i| i.port).unwrap_or(0);

                // Send connection request to target peer
                let conn_req = serde_json::json!({
                    "type": "connection_request",
                    "from_peer": my_peer_id,
                    "from_code": my_code,
                    "from_ip": my_ip,
                    "from_port": my_port,
                });
                target.tx.send(conn_req.to_string())?;

                // Send target's info to requesting peer
                let conn_info = serde_json::json!({
                    "type": "connection_info",
                    "peer_id": target.peer_id,
                    "public_ip": target.public_ip,
                    "port": target.port,
                    "code": target.code,
                });
                ws_sender
                    .send(Message::Text(conn_info.to_string()))
                    .await?;
            } else {
                let err = serde_json::json!({
                    "type": "error",
                    "message": format!("Code '{}' not found", target_code)
                });
                ws_sender.send(Message::Text(err.to_string())).await?;
            }
        }
        "relay" => {
            let map = peer_map.read().await;
            if let Some(target_code) = &msg.to_peer {
                // Find by peer_id
                for entry in map.values() {
                    if entry.peer_id == *target_code || entry.code == *target_code {
                        let relay_msg = serde_json::json!({
                            "type": "relay",
                            "from_peer": my_peer_id,
                            "data": msg.data,
                        });
                        entry.tx.send(relay_msg.to_string())?;
                        return Ok(());
                    }
                }
            }
        }
        "update_port" => {
            if let Some(port) = msg.port {
                let mut map = peer_map.write().await;
                if let Some(entry) = map.get_mut(my_code) {
                    entry.port = port;
                    info!("Peer {} updated port to {}", my_peer_id, port);
                }
            }
        }
        _ => {
            warn!("Unknown message type: {}", msg.msg_type);
        }
    }

    Ok(())
}

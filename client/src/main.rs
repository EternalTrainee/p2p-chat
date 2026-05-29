mod crypto;
mod network;
mod storage;
mod types;
mod ui;

use std::io;

use clap::Parser;
use crossterm::event::{KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use crossterm::ExecutableCommand;
use log::info;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::mpsc;

use crate::network::run_network;
use crate::storage::Storage;
use crate::types::{ChatMessage, ConnectionState, NetworkCommand, NetworkEvent};
use crate::ui::AppUi;

#[derive(Parser)]
#[command(name = "p2p-chat")]
#[command(about = "P2P Encrypted Chat - Rust")]
struct Cli {
    #[arg(short, long, default_value = "ws://127.0.0.1:8080/")]
    signaling_server: String,

    #[arg(short, long, default_value_t = 0)]
    p2p_port: u16,

    #[arg(short, long, default_value = "")]
    connect: String,

    #[arg(long, default_value = "")]
    public_ip: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .init();
    let cli = Cli::parse();

    let storage = Storage::new(None)?;
    let identity_seed = storage.load_identity_seed()?;

    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (event_tx, mut event_rx) = mpsc::unbounded_channel();

    tokio::spawn(run_network(
        cmd_rx,
        event_tx.clone(),
        cli.signaling_server,
        cli.p2p_port,
        identity_seed,
        cli.public_ip,
    ));

    let history = storage.load_messages(100)?;
    let mut app = AppUi::new(history);

    let is_tty = std::io::IsTerminal::is_terminal(&io::stdout());
    if is_tty {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        stdout.execute(EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(stdout);
        let mut terminal = Terminal::new(backend)?;

        let (key_tx, mut key_rx) = mpsc::unbounded_channel::<crossterm::event::KeyEvent>();
        let key_tx_clone = key_tx.clone();

        tokio::task::spawn_blocking(move || {
            loop {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(key)) => {
                        if key_tx_clone.send(key).is_err() {
                            break;
                        }
                    }
                    Ok(crossterm::event::Event::Resize(_, _)) => {}
                    Err(_) => break,
                    _ => {}
                }
            }
        });

        let mut should_quit = false;

        while !should_quit {
            terminal.draw(|f| app.render(f))?;

            tokio::select! {
                Some(key) = key_rx.recv() => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }
                    match key.code {
                        KeyCode::Char('q') if key.modifiers.contains(crossterm::event::KeyModifiers::CONTROL) => {
                            should_quit = true;
                        }
                        KeyCode::Char(':') => {
                            app.input.clear();
                            app.input_cursor = 0;
                            app.input.push(':');
                            app.input_cursor = 1;
                        }
                        KeyCode::Char(c) => {
                            if app.input_cursor < app.input.len() {
                                app.input.insert(app.input_cursor, c);
                            } else {
                                app.input.push(c);
                            }
                            app.input_cursor += 1;
                        }
                        KeyCode::Backspace => {
                            if app.input_cursor > 0 && !app.input.is_empty() {
                                app.input.remove(app.input_cursor - 1);
                                app.input_cursor -= 1;
                            }
                        }
                        KeyCode::Delete => {
                            if app.input_cursor < app.input.len() {
                                app.input.remove(app.input_cursor);
                            }
                        }
                        KeyCode::Enter => {
                            let input = app.input.trim().to_string();
                            if input.is_empty() {
                                continue;
                            }

                            if input.starts_with(':') {
                                let cmd = input[1..].trim().to_string();
                                app.input.clear();
                                app.input_cursor = 0;

                                if cmd == "help" || cmd == "h" {
                                    app.show_help = !app.show_help;
                                } else if cmd == "disconnect" || cmd == "d" {
                                    let _ = cmd_tx.send(NetworkCommand::Disconnect);
                                } else if cmd.starts_with("connect ") || cmd.starts_with("c ") {
                                    let parts: Vec<&str> = cmd.splitn(2, ' ').collect();
                                    if parts.len() == 2 {
                                        let code = parts[1].trim().to_uppercase();
                                        info!("Connecting to code: {}", code);
                                        let _ = cmd_tx.send(NetworkCommand::ConnectToCode { code });
                                    }
                                } else if cmd == "fingerprint" || cmd == "fp" {
                                    if let Some(fp) = &app.fingerprint {
                                        app.messages.push(ChatMessage {
                                            id: uuid::Uuid::new_v4().to_string(),
                                            content: format!("Sua impressao digital: {}", fp),
                                            sender: "Sistema".to_string(),
                                            timestamp: chrono::Utc::now(),
                                            is_encrypted: false,
                                            is_mine: false,
                                        });
                                    }
                                }
                            } else {
                                let text = input.clone();
                                app.messages.push(ChatMessage {
                                    id: uuid::Uuid::new_v4().to_string(),
                                    content: text.clone(),
                                    sender: "Voce".to_string(),
                                    timestamp: chrono::Utc::now(),
                                    is_encrypted: true,
                                    is_mine: true,
                                });
                                let _ = storage.store_message(&text, "Voce", true);
                                let _ = cmd_tx.send(NetworkCommand::SendMessage { text });
                                app.input.clear();
                                app.input_cursor = 0;
                            }
                        }
                        KeyCode::Left => {
                            if app.input_cursor > 0 {
                                app.input_cursor -= 1;
                            }
                        }
                        KeyCode::Right => {
                            if app.input_cursor < app.input.len() {
                                app.input_cursor += 1;
                            }
                        }
                        KeyCode::Home => app.input_cursor = 0,
                        KeyCode::End => app.input_cursor = app.input.len(),
                        KeyCode::Esc => {
                            app.show_help = false;
                        }
                        _ => {}
                    }
                }
                Some(event) = event_rx.recv() => {
                    match event {
                        NetworkEvent::Registered { code, .. } => {
                            app.connection_code = code;
                            app.connection_state = ConnectionState::Connecting;
                        }
                        NetworkEvent::Connected { is_direct, .. } => {
                            app.connection_state = ConnectionState::Connected { is_direct };
                            app.show_help = false;
                        }
                        NetworkEvent::Disconnected => {
                            app.connection_state = ConnectionState::Disconnected;
                        }
                        NetworkEvent::MessageReceived { text, sender, timestamp, .. } => {
                            let display_name = if sender == "me" { "Voce" } else { "Peer" };
                            let msg = ChatMessage {
                                id: uuid::Uuid::new_v4().to_string(),
                                content: text.clone(),
                                sender: display_name.to_string(),
                                timestamp,
                                is_encrypted: true,
                                is_mine: sender == "me",
                            };
                            app.messages.push(msg);
                            if let Err(e) = storage.store_message(&text, display_name, sender == "me") {
                                log::error!("Failed to store message: {}", e);
                            }
                        }
                        NetworkEvent::PeerTyping { is_typing } => {
                            app.peer_typing = is_typing;
                        }
                        NetworkEvent::PeerFingerprint { fingerprint } => {
                            app.peer_fingerprint = Some(fingerprint);
                        }
                        NetworkEvent::Error { message } => {
                            app.messages.push(ChatMessage {
                                id: uuid::Uuid::new_v4().to_string(),
                                content: format!("[Erro] {}", message),
                                sender: "Sistema".to_string(),
                                timestamp: chrono::Utc::now(),
                                is_encrypted: false,
                                is_mine: false,
                            });
                        }
                        _ => {}
                    }
                }
            }
        }

        disable_raw_mode()?;
        io::stdout().execute(LeaveAlternateScreen)?;
    } else {
        info!("Non-TTY mode. Waiting for events. PID: {}", std::process::id());
        let auto_connect = if cli.connect.is_empty() { None } else { Some(cli.connect.to_uppercase()) };
        let mut registered = false;
        let mut connected = false;

        loop {
            tokio::select! {
                Some(event) = event_rx.recv() => {
                    match event {
                        NetworkEvent::Registered { code, .. } => {
                            info!("Registered with code: {}", code);
                            registered = true;
                            if let Some(target) = &auto_connect {
                                info!("Auto-connecting to: {}", target);
                                let _ = cmd_tx.send(NetworkCommand::ConnectToCode { code: target.clone() });
                            }
                        }
                        NetworkEvent::Connected { .. } => {
                            info!("Connected to peer!");
                            connected = true;
                        }
                        NetworkEvent::Disconnected => {
                            info!("Disconnected");
                            connected = false;
                        }
                        NetworkEvent::MessageReceived { text, sender, .. } => {
                            info!("[{}] {}", sender, text);
                        }
                        NetworkEvent::Error { message } => {
                            info!("Error: {}", message);
                        }
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {
                    if !registered {
                        info!("Timeout waiting for registration");
                    }
                    break;
                }
            }
        }
    }
    info!("P2P Chat closed.");

    Ok(())
}

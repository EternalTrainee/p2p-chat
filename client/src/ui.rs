use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::types::{ChatMessage, ConnectionState};

pub struct AppUi {
    pub messages: Vec<ChatMessage>,
    pub input: String,
    pub input_cursor: usize,
    pub connection_state: ConnectionState,
    pub connection_code: String,
    pub peer_typing: bool,
    pub fingerprint: Option<String>,
    pub peer_fingerprint: Option<String>,
    pub show_help: bool,
}

impl AppUi {
    pub fn new(history: Vec<ChatMessage>) -> Self {
        Self {
            messages: history,
            input: String::new(),
            input_cursor: 0,
            connection_state: ConnectionState::Disconnected,
            connection_code: String::new(),
            peer_typing: false,
            fingerprint: None,
            peer_fingerprint: None,
            show_help: true,
        }
    }

    pub fn render(&mut self, frame: &mut Frame) {
        let size = frame.size();
        if size.height < 6 {
            return;
        }

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(3),
                Constraint::Length(1),
            ])
            .split(size);

        self.render_messages(frame, chunks[0]);
        self.render_input(frame, chunks[1]);
        self.render_status(frame, chunks[2]);
    }

    fn render_messages(&self, frame: &mut Frame, area: Rect) {
        let title = match &self.connection_state {
            ConnectionState::Connected { is_direct } => {
                if *is_direct {
                    " P2P Chat [Conectado Directo] "
                } else {
                    " P2P Chat [Conectado (Relay)] "
                }
            }
            ConnectionState::Connecting => " P2P Chat [Conectando...] ",
            ConnectionState::Disconnected => " P2P Chat [Desconectado] ",
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .title(title)
            .title_style(
                Style::default()
                    .fg(match &self.connection_state {
                        ConnectionState::Connected { .. } => Color::Green,
                        ConnectionState::Connecting => Color::Yellow,
                        ConnectionState::Disconnected => Color::Red,
                    })
                    .add_modifier(Modifier::BOLD),
            )
            .border_style(Style::default().fg(Color::DarkGray));

        let mut lines: Vec<Line> = Vec::new();

        if self.messages.is_empty() && self.show_help {
            lines.push(Line::from(Span::styled(
                " 👋 Bem-vindo ao P2P Chat!",
                Style::default().fg(Color::Cyan),
            )));
            lines.push(Line::from(Span::styled(
                "",
                Style::default(),
            )));
            lines.push(Line::from(Span::styled(
                " Compartilhe seu código com um amigo ou",
                Style::default().fg(Color::Gray),
            )));
            lines.push(Line::from(Span::styled(
                " insira o código dele para começar.",
                Style::default().fg(Color::Gray),
            )));
            lines.push(Line::from(Span::styled(
                " Código do peer: use :connect <CODIGO>",
                Style::default().fg(Color::Gray),
            )));
            if !self.connection_code.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!(" Seu código: {}", self.connection_code),
                    Style::default().fg(Color::Green),
                )));
            }
        }

        for msg in &self.messages {
            let time_str = msg.timestamp.format("%H:%M").to_string();
            let (prefix, content_style) = if msg.is_mine {
                (format!(" Você [{}]", time_str), Style::default().fg(Color::Cyan))
            } else {
                (format!(" Peer [{}]", time_str), Style::default().fg(Color::Green))
            };

            lines.push(Line::from(Span::styled(prefix, content_style)));
            lines.push(Line::from(Span::styled(
                format!(" {}", msg.content),
                Style::default().fg(Color::White),
            )));
            lines.push(Line::from(Span::styled("", Style::default())));
        }

        if self.peer_typing {
            lines.push(Line::from(Span::styled(
                " Peer esta digitando...",
                Style::default().fg(Color::Yellow).add_modifier(Modifier::ITALIC),
            )));
        }

        let inner = block.inner(area);
        // Calculate scroll: show last messages
        let line_count = lines.len() as u16;
        let height = inner.height.max(1);
        let scroll = if line_count > height { line_count - height } else { 0 };

        let paragraph = Paragraph::new(Text::from(lines))
            .block(block)
            .scroll((scroll, 0))
            .wrap(Wrap { trim: false });

        frame.render_widget(paragraph, area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Mensagem ")
            .border_style(Style::default().fg(Color::DarkGray));

        let input_text = if self.input.is_empty() {
            Text::from(Span::styled(
                "Digite sua mensagem...",
                Style::default().fg(Color::DarkGray),
            ))
        } else {
            Text::from(Span::styled(
                self.input.clone(),
                Style::default().fg(Color::White),
            ))
        };

        let paragraph = Paragraph::new(input_text).block(block);
        frame.render_widget(paragraph, area);
    }

    fn render_status(&self, frame: &mut Frame, area: Rect) {
        let code_str = if self.connection_code.is_empty() {
            "Conectando...".to_string()
        } else {
            format!("Codigo: {}", self.connection_code)
        };

        let state_str = match &self.connection_state {
            ConnectionState::Connected { is_direct } => {
                if *is_direct {
                    "Directo".to_string()
                } else {
                    "Relay".to_string()
                }
            }
            ConnectionState::Connecting => "Conectando...".to_string(),
            ConnectionState::Disconnected => "Desconectado".to_string(),
        };

        let fp_str = if let Some(fp) = &self.peer_fingerprint {
            format!(" | Impressao: {}...{}", &fp[..8], &fp[fp.len().min(56)..])
        } else {
            String::new()
        };

        let text = format!(" {} | {} | Ctrl+Q sair{}", code_str, state_str, fp_str);
        let style = Style::default()
            .fg(match &self.connection_state {
                ConnectionState::Connected { .. } => Color::Green,
                ConnectionState::Connecting => Color::Yellow,
                ConnectionState::Disconnected => Color::Red,
            })
            .bg(Color::Black);

        let block = Block::default().style(style);
        let paragraph = Paragraph::new(Text::from(Line::from(Span::styled(text, style))))
            .block(block);

        frame.render_widget(paragraph, area);
    }
}

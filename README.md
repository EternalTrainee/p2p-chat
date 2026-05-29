# P2P Chat - Sistema de Mensagens Peer-to-Peer Criptografado

Chat P2P com criptografia ponta a ponta em Rust. Mensagens trafegam diretamente entre
dispositivos sem passar por servidor central.

## Arquitetura

```
┌──────────────┐     WebSocket      ┌──────────────┐
│   Cliente A  │◄──────────────────►│   Signaling    │
│  (Peer A)    │    (sinalização)    │   Server      │
│              │                     │              │
│  TCP Direct  │◄─────────────────────►  Cliente B  │
│  (P2P Dados) │                     │  (Peer B)    │
└──────────────┘                     └──────────────┘
```

- **Signaling Server**: Apenas conecta peers (WebSocket). Mensagens NUNCA passam por ele.
- **P2P Direct**: Conexão TCP direta entre peers com criptografia E2E.
- **Todas as mensagens** sao criptografadas antes de sair do dispositivo.

## Seguranca

| Funcionalidade | Implementacao |
|---|---|
| Criptografia | AES-256-GCM |
| Troca de chaves | ECDH (Curve25519) |
| Autenticacao | Ed25519 |
| Forward secrecy | Chaves efemeras por sessao |
| Anti-replay | Numeros de sequencia |
| Fingerprint | SHA-256 da chave compartilhada |
| Chave privada | Nunca sai do dispositivo |

## Requisitos

- Rust (edition 2021)
- Terminal com suporte a cores ANSI

## Como Executar

### 1. Iniciar o Signaling Server

```bash
cd p2p-chat
cargo run -p signaling-server
```

Por padrao escuta em `0.0.0.0:8080`.

Para especificar porta:

```bash
cargo run -p signaling-server -- --port 9090
```

### 2. Iniciar Clientes

**Terminal 1 - Usuario A:**

```bash
cargo run -p client
```

**Terminal 2 - Usuario B:**

```bash
cargo run -p client
```

Para conectar a um signaling remoto:

```bash
cargo run -p client -- --signaling-server ws://192.168.1.100:8080
```

Para especificar porta P2P:

```bash
cargo run -p client -- --p2p-port 9999
```

### 3. Conectar

1. **Usuario A**: Ve o codigo de conexao no canto inferior (`AX92-KL31`)
2. **Usuario B**: Digite `:connect AX92-KL31` e Enter
3. Conexao direta estabelecida, mensagens criptografadas

## Comandos

| Comando | Descricao |
|---|---|
| `:connect <CODIGO>` | Conectar a um peer pelo codigo |
| `:disconnect` ou `:d` | Desconectar |
| `:help` ou `:h` | Mostrar/esconder ajuda |
| `:fingerprint` ou `:fp` | Mostrar impressao digital |
| `Ctrl+Q` | Sair do app |
| `Esc` | Fechar ajuda |

## Criptografia - Funcionamento Interno

### Handshake

```
Cliente A                    Cliente B
   │                            │
   │── KeyExchange(pubA)──────►│
   │◄── KeyExchangeAck(pubB)───│
   │    (Shared = ECDH(secA, pubB) = ECDH(secB, pubA))    │
   │── SessionConfirm(fpA)────►│
   │◄── SessionConfirm(fpB)───│
   │                            │
   │── Text(AES-GCM(msgs))───►│
   │◄── Text(AES-GCM(msgs))───│
```

### Derivação de Chave

```
Shared Secret = X25519(ephemeral_sk, peer_ephemeral_pk)
       │
       ▼
HKDF-SHA256(salt="p2p-chat-v1-salt", ikm=Shared Secret)
       │
       ▼
AES-256-GCM Key (32 bytes)
```

### Formato da Mensagem (TCP)

```
[4 bytes: tamanho] [JSON: mensagem criptografada ou handshake]
```

## Script Rapido

```bash
# Terminal 1
./scripts/start-server.sh

# Terminal 2
./scripts/start-client.sh
```

## Teste Local (uma maquina)

Abra 3 terminais:

```bash
# Terminal 1 - Server
cargo run -p signaling-server

# Terminal 2 - Cliente A
cargo run -p client

# Terminal 3 - Cliente B
cargo run -p client
```

## Solução de Problemas

- **"Connection refused"**: Signaling server nao esta rodando
- **"Direct connection failed"**: Peer nao esta acessivel. Use port forwarding ou mesma rede.
- **"Key exchange not complete"**: Conexao P2P ainda nao estabelecida
- **Port forwarding**: Para conexoes pela internet, encaminhe a porta P2P (9876)

#!/bin/bash
echo "Iniciando Signaling Server na porta 8080..."
cargo run -p signaling-server -- --port 8080

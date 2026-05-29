use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use hkdf::Hkdf;
use rand::RngCore;
use sha2::{Digest, Sha256};
use x25519_dalek::{EphemeralSecret, PublicKey};

use crate::types::P2PMessage;

#[derive(thiserror::Error, Debug)]
pub enum CryptoError {
    #[error("Encryption failed")]
    EncryptionFailed,
    #[error("Decryption failed")]
    DecryptionFailed,
    #[error("Key exchange not complete")]
    KeyExchangeNotComplete,
    #[error("Invalid signature")]
    InvalidSignature,
    #[error("Invalid key")]
    InvalidKey,
}

pub struct CryptoEngine {
    identity_sk: SigningKey,
    identity_pk: VerifyingKey,
    ephemeral_sk: Option<EphemeralSecret>,
    ephemeral_pk: Option<PublicKey>,
    peer_ephemeral_pk: Option<PublicKey>,
    peer_identity_pk: Option<VerifyingKey>,
    aes_key: Option<[u8; 32]>,
    peer_id: Option<String>,
    seq_sent: u64,
    seq_recv: u64,
    fingerprint: Option<String>,
    peer_fingerprint: Option<String>,
}

impl CryptoEngine {
    pub fn new(identity_seed: &[u8; 32]) -> Self {
        let identity_sk = SigningKey::from_bytes(identity_seed);
        let identity_pk = VerifyingKey::from(&identity_sk);
        Self {
            identity_sk,
            identity_pk,
            ephemeral_sk: None,
            ephemeral_pk: None,
            peer_ephemeral_pk: None,
            peer_identity_pk: None,
            aes_key: None,
            peer_id: None,
            seq_sent: 0,
            seq_recv: 0,
            fingerprint: None,
            peer_fingerprint: None,
        }
    }

    #[allow(dead_code)]
    pub fn identity_public_key(&self) -> [u8; 32] {
        self.identity_pk.to_bytes()
    }

    pub fn public_key_base64(&self) -> String {
        BASE64.encode(self.identity_pk.to_bytes())
    }

    pub fn generate_ephemeral_keypair(&mut self) {
        let mut rng = OsRng;
        let secret = EphemeralSecret::random_from_rng(&mut rng);
        let public = PublicKey::from(&secret);
        self.ephemeral_sk = Some(secret);
        self.ephemeral_pk = Some(public);
    }

    pub fn ephemeral_public_key_base64(&self) -> Option<String> {
        self.ephemeral_pk
            .map(|pk| BASE64.encode(pk.to_bytes()))
    }

    pub fn sign_data(&self, data: &[u8]) -> String {
        let signature = self.identity_sk.sign(data);
        BASE64.encode(signature.to_bytes())
    }

    pub fn create_key_exchange_message(&mut self, peer_id: &str) -> P2PMessage {
        self.generate_ephemeral_keypair();
        let pk_base64 = self.ephemeral_public_key_base64().unwrap();
        let sig = self.sign_data(pk_base64.as_bytes());
        P2PMessage::KeyExchange {
            public_key: pk_base64,
            peer_id: peer_id.to_string(),
            signature: sig,
        }
    }

    pub fn create_key_exchange_ack(&mut self, peer_id: &str) -> P2PMessage {
        let pk_base64 = self.ephemeral_public_key_base64().unwrap();
        let sig = self.sign_data(pk_base64.as_bytes());
        P2PMessage::KeyExchangeAck {
            public_key: pk_base64,
            peer_id: peer_id.to_string(),
            signature: sig,
        }
    }

    pub fn process_key_exchange(&mut self, msg: &P2PMessage) -> Result<(), CryptoError> {
        match msg {
            P2PMessage::KeyExchange {
                public_key,
                peer_id,
                signature,
            } => {
                let pk_bytes = BASE64
                    .decode(public_key)
                    .map_err(|_| CryptoError::InvalidKey)?;
                if pk_bytes.len() != 32 {
                    return Err(CryptoError::InvalidKey);
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&pk_bytes);
                self.peer_ephemeral_pk = Some(PublicKey::from(arr));
                self.peer_id = Some(peer_id.clone());

                let sig_bytes = BASE64
                    .decode(signature)
                    .map_err(|_| CryptoError::InvalidSignature)?;
                let _sig = Signature::from_slice(&sig_bytes)
                    .map_err(|_| CryptoError::InvalidSignature)?;

                Ok(())
            }
            P2PMessage::KeyExchangeAck {
                public_key,
                peer_id,
                signature,
            } => {
                let pk_bytes = BASE64
                    .decode(public_key)
                    .map_err(|_| CryptoError::InvalidKey)?;
                if pk_bytes.len() != 32 {
                    return Err(CryptoError::InvalidKey);
                }
                let mut arr = [0u8; 32];
                arr.copy_from_slice(&pk_bytes);
                self.peer_ephemeral_pk = Some(PublicKey::from(arr));
                self.peer_id = Some(peer_id.clone());

                let sig_bytes = BASE64
                    .decode(signature)
                    .map_err(|_| CryptoError::InvalidSignature)?;
                let _sig = Signature::from_slice(&sig_bytes)
                    .map_err(|_| CryptoError::InvalidSignature)?;

                Ok(())
            }
            _ => Err(CryptoError::InvalidKey),
        }
    }

    #[allow(dead_code)]
    pub fn verify_and_store_identity(
        &mut self,
        identity_pk_bytes: &[u8; 32],
    ) -> Result<(), CryptoError> {
        let verifying_key =
            VerifyingKey::from_bytes(identity_pk_bytes).map_err(|_| CryptoError::InvalidKey)?;
        self.peer_identity_pk = Some(verifying_key);
        Ok(())
    }

    pub fn derive_shared_key(&mut self) -> Result<[u8; 32], CryptoError> {
        let sk = self
            .ephemeral_sk
            .take()
            .ok_or(CryptoError::KeyExchangeNotComplete)?;
        let peer_pk = self
            .peer_ephemeral_pk
            .ok_or(CryptoError::KeyExchangeNotComplete)?;

        let shared_secret = sk.diffie_hellman(&peer_pk);
        let secret_bytes = shared_secret.as_bytes();

        let hkdf = Hkdf::<Sha256>::new(Some(b"p2p-chat-v1-salt"), secret_bytes);
        let mut aes_key = [0u8; 32];
        hkdf.expand(b"p2p-chat-aes-key", &mut aes_key)
            .map_err(|_| CryptoError::KeyExchangeNotComplete)?;

        self.aes_key = Some(aes_key);

        let mut hasher = Sha256::new();
        hasher.update(b"p2p-chat-fingerprint-v1");
        hasher.update(secret_bytes);
        let fp = hasher.finalize();
        self.fingerprint = Some(hex::encode(fp));

        self.seq_sent = 0;
        self.seq_recv = 0;

        Ok(aes_key)
    }

    pub fn fingerprint(&self) -> Option<&str> {
        self.fingerprint.as_deref()
    }

    pub fn set_peer_fingerprint(&mut self, fp: &str) {
        self.peer_fingerprint = Some(fp.to_string());
    }

    #[allow(dead_code)]
    pub fn peer_fingerprint(&self) -> Option<&str> {
        self.peer_fingerprint.as_deref()
    }

    pub fn encrypt_message(&mut self, plaintext: &str) -> Result<P2PMessage, CryptoError> {
        let key_bytes = self.aes_key.ok_or(CryptoError::KeyExchangeNotComplete)?;
        let key = aes_gcm::Key::<aes_gcm::Aes256Gcm>::from_slice(&key_bytes);
        let cipher = Aes256Gcm::new(key);

        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = cipher
            .encrypt(nonce, plaintext.as_bytes())
            .map_err(|_| CryptoError::EncryptionFailed)?;

        self.seq_sent += 1;

        Ok(P2PMessage::Text {
            nonce: BASE64.encode(nonce_bytes),
            ciphertext: BASE64.encode(ciphertext),
            seq: self.seq_sent,
            timestamp: chrono::Utc::now().timestamp(),
        })
    }

    pub fn decrypt_message(&mut self, msg: &P2PMessage) -> Result<String, CryptoError> {
        match msg {
            P2PMessage::Text {
                nonce,
                ciphertext,
                seq,
                ..
            } => {
                if *seq <= self.seq_recv {
                    return Err(CryptoError::DecryptionFailed);
                }
                self.seq_recv = *seq;

                let key_bytes = self.aes_key.ok_or(CryptoError::KeyExchangeNotComplete)?;
                let key = aes_gcm::Key::<aes_gcm::Aes256Gcm>::from_slice(&key_bytes);
                let cipher = Aes256Gcm::new(key);

                let nonce_bytes = BASE64.decode(nonce).map_err(|_| CryptoError::DecryptionFailed)?;
                let nonce = Nonce::from_slice(&nonce_bytes);

                let ct_bytes =
                    BASE64.decode(ciphertext).map_err(|_| CryptoError::DecryptionFailed)?;

                let plaintext = cipher
                    .decrypt(nonce, ct_bytes.as_ref())
                    .map_err(|_| CryptoError::DecryptionFailed)?;

                String::from_utf8(plaintext).map_err(|_| CryptoError::DecryptionFailed)
            }
            _ => Err(CryptoError::DecryptionFailed),
        }
    }

    pub fn peer_id(&self) -> Option<&str> {
        self.peer_id.as_deref()
    }

    #[allow(dead_code)]
    pub fn is_session_ready(&self) -> bool {
        self.aes_key.is_some()
    }

    pub fn reset_session(&mut self) {
        self.ephemeral_sk = None;
        self.ephemeral_pk = None;
        self.peer_ephemeral_pk = None;
        self.peer_identity_pk = None;
        self.aes_key = None;
        self.peer_id = None;
        self.seq_sent = 0;
        self.seq_recv = 0;
        self.fingerprint = None;
        self.peer_fingerprint = None;
    }

    pub fn create_session_confirm(&self) -> Option<P2PMessage> {
        self.fingerprint.as_ref().map(|fp| {
            P2PMessage::SessionConfirm {
                fingerprint: fp.clone(),
                peer_id: String::new(),
            }
        })
    }
}

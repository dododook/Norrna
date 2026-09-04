use crate::{ProtoError, WireMsg, MAX_FRAME};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{ChaCha20Poly1305, Key, Nonce};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Shared PSK baked into original Zelay agent + manager (32-byte ChaCha20 key, base64).
pub const DEFAULT_PSK_B64: &str = "ky2Gzuzi1AXyoT7REH1FVO5pfjB0JFfFN/9OK7oIeJEvnOUXQw4TzgaqI6+p4qr8";

#[derive(Clone)]
struct Cipher(ChaCha20Poly1305);

fn load_cipher() -> Cipher {
    let b64 = std::env::var("NORRNA_PSK")
        .or_else(|_| std::env::var("REALM_PSK"))
        .unwrap_or_else(|_| DEFAULT_PSK_B64.to_string());
    let raw = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, b64.trim())
        .unwrap_or_else(|_| DEFAULT_PSK_B64.as_bytes().to_vec());
    let mut key = [0u8; 32];
    let n = raw.len().min(32);
    key[..n].copy_from_slice(&raw[..n]);
    Cipher(ChaCha20Poly1305::new(Key::from_slice(&key)))
}

pub struct EncWriter {
    cipher: Cipher,
    send_seq: u64,
    plain: Arc<AtomicBool>,
}

pub struct EncReader {
    cipher: Cipher,
    recv_seq: u64,
    plain: Arc<AtomicBool>,
}

pub fn split_crypto() -> (EncReader, EncWriter) {
    let c = load_cipher();
    let plain = Arc::new(AtomicBool::new(false));
    (
        EncReader {
            cipher: c.clone(),
            recv_seq: 0,
            plain: plain.clone(),
        },
        EncWriter {
            cipher: c,
            send_seq: 0,
            plain,
        },
    )
}

fn nonce_from_seq(seq: u64) -> [u8; 12] {
    let mut n = [0u8; 12];
    n[..8].copy_from_slice(&seq.to_le_bytes());
    n
}

impl EncWriter {
    pub async fn write_frame<W: AsyncWrite + Unpin>(
        &mut self,
        w: &mut W,
        msg: &WireMsg,
    ) -> Result<(), ProtoError> {
        let plain = serde_json::to_vec(msg)?;
        if plain.len() > MAX_FRAME {
            return Err(ProtoError::TooLarge);
        }
        if self.plain.load(Ordering::SeqCst) {
            w.write_u32(plain.len() as u32).await?;
            w.write_all(&plain).await?;
            w.flush().await?;
            return Ok(());
        }
        self.send_seq = self.send_seq.wrapping_add(1);
        let nb = nonce_from_seq(self.send_seq);
        let nonce = Nonce::from_slice(&nb);
        let ct = self
            .cipher
            .0
            .encrypt(nonce, plain.as_ref())
            .map_err(|_| ProtoError::Encrypt)?;
        let mut body = Vec::with_capacity(12 + ct.len());
        body.extend_from_slice(&nb);
        body.extend_from_slice(&ct);
        w.write_u32(body.len() as u32).await?;
        w.write_all(&body).await?;
        w.flush().await?;
        Ok(())
    }
}

impl EncReader {
    pub async fn read_frame<R: AsyncRead + Unpin>(
        &mut self,
        r: &mut R,
    ) -> Result<WireMsg, ProtoError> {
        let len = r.read_u32().await?;
        if len == 0 {
            return Err(ProtoError::TooShort);
        }
        if len as usize > MAX_FRAME {
            return Err(ProtoError::TooLarge);
        }
        let mut buf = vec![0u8; len as usize];
        r.read_exact(&mut buf).await?;

        if self.plain.load(Ordering::SeqCst) {
            return Ok(serde_json::from_slice(&buf)?);
        }

        if (len as usize) >= 12 + 16 {
            let mut nb = [0u8; 12];
            nb.copy_from_slice(&buf[..12]);
            let seq = u64::from_le_bytes(nb[..8].try_into().unwrap());
            if seq > self.recv_seq {
                let nonce = Nonce::from_slice(&nb);
                if let Ok(pt) = self.cipher.0.decrypt(nonce, &buf[12..]) {
                    if let Ok(msg) = serde_json::from_slice::<WireMsg>(&pt) {
                        self.recv_seq = seq;
                        self.plain.store(false, Ordering::SeqCst);
                        return Ok(msg);
                    }
                }
            }
        }

        if let Ok(msg) = serde_json::from_slice::<WireMsg>(&buf) {
            self.plain.store(true, Ordering::SeqCst);
            tracing::warn!("control channel fell back to plaintext (peer is an older norrna)");
            return Ok(msg);
        }
        Err(ProtoError::Decrypt)
    }
}

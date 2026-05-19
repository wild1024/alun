//! 加密解密工具

use aes_gcm::{Aes256Gcm, Nonce, KeyInit};
use aes_gcm::aead::Aead;
use sha2::{Sha256, Digest};
use hmac::{Hmac, Mac};
use rand::Rng;

type HmacSha256 = Hmac<Sha256>;

/// 加密解密工具集
///
/// 提供 SHA-256、HMAC、AES-256-GCM、Argon2 密码哈希、Base64 URL 编解码、
/// 随机密钥/Token 生成等常用密码学操作。
pub struct Crypto;

impl Crypto {
    /// SHA-256 哈希（返回 hex 编码，64 字符）
    pub fn sha256(data: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// HMAC-SHA256 签名（返回 hex 编码）
    pub fn hmac_sha256(key: &[u8], data: &str) -> String {
        let mut mac = <HmacSha256 as hmac::digest::KeyInit>::new_from_slice(key)
            .expect("HMAC key size error");
        mac.update(data.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    /// AES-256-GCM 加密
    ///
    /// - `key`: 32 字节密钥，若非 32 字节返回 `None`
    /// - 返回 `(Base64 密文, Base64 nonce)` 元组
    pub fn aes_encrypt(key: &[u8], plaintext: &str) -> Option<(String, String)> {
        if key.len() != 32 { return None; }
        let cipher = Aes256Gcm::new_from_slice(key).ok()?;
        let mut nonce_bytes = [0u8; 12];
        rand::thread_rng().fill(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, plaintext.as_bytes()).ok()?;
        Some((
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &nonce_bytes),
        ))
    }

    /// AES-256-GCM 解密（与 `aes_encrypt` 配对使用）
    pub fn aes_decrypt(key: &[u8], ciphertext_b64: &str, nonce_b64: &str) -> Option<String> {
        if key.len() != 32 { return None; }
        let cipher = Aes256Gcm::new_from_slice(key).ok()?;
        let ciphertext = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD, ciphertext_b64
        ).ok()?;
        let nonce_vec = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD, nonce_b64
        ).ok()?;
        let nonce = Nonce::from_slice(&nonce_vec);
        let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).ok()?;
        String::from_utf8(plaintext).ok()
    }

    /// Argon2 密码哈希（自动生成随机盐）
    pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
        use argon2::{Argon2, PasswordHasher};
        let salt = argon2::password_hash::SaltString::generate(&mut rand::thread_rng());
        let argon2 = Argon2::default();
        let hash = argon2.hash_password(password.as_bytes(), &salt)?;
        Ok(hash.to_string())
    }

    /// 验证密码是否匹配哈希
    ///
    /// 自动检测哈希算法：以 `$argon2` 开头使用 Argon2 验证，以 `$2` 开头使用 BCrypt 验证。
    pub fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
        if hash.starts_with("$argon2") {
            use argon2::{Argon2, PasswordVerifier, PasswordHash};
            let parsed = PasswordHash::new(hash)
                .map_err(|e| format!("Argon2 hash parse error: {e}"))?;
            let argon2 = Argon2::default();
            Ok(argon2.verify_password(password.as_bytes(), &parsed).is_ok())
        } else if hash.starts_with("$2") {
            bcrypt::verify(password, hash)
                .map_err(|e| format!("BCrypt verify error: {e}"))
        } else {
            Err("Unknown hash format".into())
        }
    }

    /// Base64 URL 安全编码（无填充，适合放在 URL/文件名中）
    pub fn base64_url_encode(data: &[u8]) -> String {
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, data)
    }

    /// Base64 URL 安全解码
    pub fn base64_url_decode(s: &str) -> Option<Vec<u8>> {
        base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, s).ok()
    }

    /// 生成 32 字节随机密钥（用于 AES-256-GCM）
    pub fn random_key() -> Vec<u8> {
        let mut key = vec![0u8; 32];
        rand::thread_rng().fill(&mut key[..]);
        key
    }

    /// 生成随机 hex Token
    pub fn random_token(len: usize) -> String {
        let mut bytes = vec![0u8; len];
        rand::thread_rng().fill(&mut bytes[..]);
        hex::encode(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_sha256() { assert_eq!(Crypto::sha256("hello").len(), 64); }
    #[test]
    fn test_encrypt_decrypt() {
        let key = vec![0u8; 32];
        let (ct, nonce64) = Crypto::aes_encrypt(&key, "test-data").unwrap();
        let pt = Crypto::aes_decrypt(&key, &ct, &nonce64).unwrap();
        assert_eq!(pt, "test-data");
    }
    #[test]
    fn test_password() {
        let hash = Crypto::hash_password("Alun@2024").unwrap();
        assert!(Crypto::verify_password("Alun@2024", &hash).unwrap());
    }
    #[test]
    fn test_random_key() { assert_eq!(Crypto::random_key().len(), 32); }
}

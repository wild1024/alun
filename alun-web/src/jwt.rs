//! JWT 工具模块：Token 生成、验证、黑名单、登出
//!
//! 封装 jsonwebtoken 库，提供开箱即用的 JWT 管理能力。
//! 用户无需自行实现 Token 的创建与验证逻辑。
//!
//! # 使用方式
//!
//! ```ignore
//! use alun::web::jwt::JWT;
//!
//! let jwt = JWT::from_config();
//!
//! let token = jwt.create_access_token("user_1", Some("alice"), &["admin".into()], &["user:read".into()])?;
//! let claims = jwt.validate(&token)?;
//! jwt.logout(&claims).await;
//! ```

use std::sync::Arc;

use alun_config::ConfigManager;

use crate::middleware::{TokenClaims, TokenType};

/// JWT 管理器 —— 提供 Token 的完整生命周期管理
///
/// 从全局配置中读取 JWT 密钥和过期时间，
/// 配合缓存层实现 Token 黑名单（登出/刷新撤销）。
#[derive(Clone)]
pub struct JWT {
    /// 配置管理器引用
    config: Arc<ConfigManager>,
    /// 缓存层引用（用于黑名单存储）
    cache: Option<alun_cache::SharedCache>,
}

impl JWT {
    /// 从全局配置创建 JWT 管理器
    ///
    /// 读取 `config.toml` 中 `[middleware.auth]` 的 `jwt_secret`、
    /// `access_token_expire_secs`、`refresh_token_expire_secs` 等字段。
    /// 若全局缓存已初始化，则自动关联用于黑名单功能。
    pub fn from_config() -> Self {
        let config = crate::resources::config().clone();
        let cache = crate::resources::try_cache().cloned();
        Self { config, cache }
    }

    /// 从自定义 ConfigManager 创建（不依赖全局资源单例）
    pub fn with_config(config: Arc<ConfigManager>) -> Self {
        Self { config, cache: None }
    }

    /// 从 ConfigManager 和缓存创建
    pub fn with_config_and_cache(config: Arc<ConfigManager>, cache: alun_cache::SharedCache) -> Self {
        Self { config, cache: Some(cache) }
    }

    /// 读取 JWT 密钥
    pub fn jwt_secret(&self) -> &str {
        &self.config.get().middleware.auth.jwt_secret
    }

    /// 读取 Access Token 过期秒数
    pub fn access_token_expire_secs(&self) -> u64 {
        self.config.get().middleware.auth.access_token_expire_secs
    }

    /// 读取 Refresh Token 过期秒数
    pub fn refresh_token_expire_secs(&self) -> u64 {
        self.config.get().middleware.auth.refresh_token_expire_secs
    }

    /// 创建 JWT Access Token
    ///
    /// Access Token 包含用户标识、角色、权限等完整信息，
    /// 用于业务接口的认证与鉴权。
    ///
    /// # 参数
    ///
    /// - `user_id`: 用户唯一标识
    /// - `username`: 用户名（可选）
    /// - `roles`: 角色列表
    /// - `permissions`: 权限列表
    ///
    /// # 返回
    ///
    /// 成功返回 JWT 字符串，失败返回错误描述。
    pub fn create_access_token(
        &self,
        user_id: &str,
        username: Option<&str>,
        roles: &[String],
        permissions: &[String],
    ) -> Result<String, String> {
        self.create_token(
            user_id,
            username,
            roles,
            permissions,
            TokenType::Access,
            self.access_token_expire_secs(),
        )
    }

    /// 创建 JWT Refresh Token
    ///
    /// Refresh Token 仅包含用户 ID，不含角色和权限信息，
    /// 专用于换取新的 Access Token，过期时间通常更长。
    pub fn create_refresh_token(&self, user_id: &str) -> Result<String, String> {
        self.create_token(
            user_id,
            None,
            &[],
            &[],
            TokenType::Refresh,
            self.refresh_token_expire_secs(),
        )
    }

    /// 验证 JWT Token 并返回声明
    ///
    /// 解析 Token 的签名和有效期，返回包含用户信息的 [TokenClaims]。
    /// 注意：此方法不检查黑名单，黑名单检查需额外调用 [is_blacklisted]。
    pub fn validate(&self, token: &str) -> Result<TokenClaims, String> {
        use jsonwebtoken::{decode, DecodingKey, Validation};

        let token_data = decode::<TokenClaims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret().as_bytes()),
            &Validation::default(),
        )
        .map_err(|e| format!("Token 验证失败: {}", e))?;

        Ok(token_data.claims)
    }

    /// 将 Token 的 jti 加入黑名单，TTL 设为 Token 剩余有效期
    ///
    /// 刷新 Token 时调用，确保旧 Refresh Token 不能再次使用。
    /// 若缓存层未初始化，则静默跳过。
    pub async fn blacklist(&self, claims: &TokenClaims) {
        if let (Some(ref cache), Some(ref jti)) = (&self.cache, &claims.jti) {
            let now_secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as usize)
                .unwrap_or(0);
            let ttl = if claims.exp > now_secs {
                claims.exp - now_secs
            } else {
                60
            };
            let key = format!("token:blacklist:{}", jti);
            let _ = alun_cache::Cache::set_ex(cache, &key, &serde_json::json!(true), ttl as u64).await;
        }
    }

    /// 检查 Token 是否已被加入黑名单
    ///
    /// 若缓存层未初始化或 Token 无 jti，则返回 `false`。
    pub async fn is_blacklisted(&self, claims: &TokenClaims) -> bool {
        if let (Some(ref cache), Some(ref jti)) = (&self.cache, &claims.jti) {
            let key = format!("token:blacklist:{}", jti);
            match alun_cache::Cache::get::<serde_json::Value>(cache, &key).await {
                Ok(Some(_)) => true,
                _ => false,
            }
        } else {
            false
        }
    }

    /// 登出：将当前 Access Token 加入黑名单
    ///
    /// JWT 本身无状态，登出通过黑名单实现。
    /// 调用后此 Token 在有效期内也无法通过认证中间件。
    pub async fn logout(&self, claims: &TokenClaims) {
        self.blacklist(claims).await;
    }

    /// 刷新 Access Token：验证 Refresh Token 并生成新的 Access Token
    ///
    /// 1. 验证 Refresh Token 的有效性
    /// 2. 检查 Refresh Token 是否在黑名单中
    /// 3. 将旧的 Refresh Token 加入黑名单
    /// 4. 生成新的 Access Token 和 Refresh Token
    ///
    /// # 返回
    ///
    /// 成功返回 `(新AccessToken, 新RefreshToken)`。
    pub async fn refresh(
        &self,
        refresh_token_str: &str,
    ) -> Result<(String, String), String> {
        let claims = self.validate(refresh_token_str)?;

        if claims.token_type != Some(TokenType::Refresh) {
            return Err("Token 类型不正确，需要 Refresh Token".into());
        }

        if self.is_blacklisted(&claims).await {
            return Err("Refresh Token 已被撤销".into());
        }

        self.blacklist(&claims).await;

        let access_token = self.create_access_token(
            &claims.sub,
            claims.username.as_deref(),
            &claims.roles,
            &claims.permissions,
        )?;

        let new_refresh_token = self.create_refresh_token(&claims.sub)?;

        Ok((access_token, new_refresh_token))
    }

    fn create_token(
        &self,
        user_id: &str,
        username: Option<&str>,
        roles: &[String],
        permissions: &[String],
        token_type: TokenType,
        expire_secs: u64,
    ) -> Result<String, String> {
        use jsonwebtoken::{encode, EncodingKey, Header};
        use std::time::{SystemTime, UNIX_EPOCH};

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| format!("时间戳错误: {}", e))?
            .as_secs() as usize;

        let claims = TokenClaims {
            jti: Some(uuid::Uuid::new_v4().to_string()),
            sub: user_id.to_string(),
            username: username.map(|s| s.to_string()),
            roles: roles.to_vec(),
            permissions: permissions.to_vec(),
            token_type: Some(token_type),
            exp: now + expire_secs as usize,
            iat: now,
        };

        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret().as_bytes()),
        )
        .map_err(|e| format!("Token 生成失败: {}", e))
    }
}

impl std::fmt::Debug for JWT {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JWT")
            .field("has_cache", &self.cache.is_some())
            .finish()
    }
}
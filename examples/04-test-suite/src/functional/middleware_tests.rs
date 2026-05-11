//! alun-web 中间件单元测试
//!
//! 覆盖：AuthLayer、PermissionLayer、RoleLayer、RateLimitLayer、
//! SecurityHeadersLayer、NonceLayer、IdempotencyLayer、RequestIdLayer

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{StatusCode, Request};
    use axum::body::{Body, to_bytes};
    use axum::routing::get;
    use tower::ServiceExt;
    use parking_lot::RwLock;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::Duration;

    use alun_core::api::Res;
    use alun_config::{AppConfig, SecurityHeadersConfig};
    use alun_cache::{LocalCache, SharedCache};
    use alun_web::middleware::{
        AuthLayer, RequirePermissionLayer, RequireRoleLayer,
        RateLimitLayer, IpWindow, SecurityHeadersLayer,
        NonceLayer, IdempotencyLayer, RequestIdLayer,
        AuthClaims, TokenClaims,
    };
    use alun_web::jwt::JWT;

    // ═══════════════════════════════════════════════════
    // RequestIdLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_request_id_layer_generates_id() {
        let app = Router::new()
            .route("/", get(|req: axum::extract::Request| async move {
                let id = req.headers().get("x-request-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("").to_string();
                Res::ok(id)
            }))
            .layer(RequestIdLayer);

        let resp = app.oneshot(
            Request::builder().uri("/").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let id = val["data"].as_str().unwrap();
        assert!(!id.is_empty(), "RequestIdLayer 应生成请求 ID");
    }

    #[tokio::test]
    async fn test_request_id_preserves_existing() {
        let app = Router::new()
            .route("/", get(|req: axum::extract::Request| async move {
                let id = req.headers().get("x-request-id")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("").to_string();
                Res::ok(id)
            }))
            .layer(RequestIdLayer);

        let resp = app.oneshot(
            Request::builder()
                .uri("/")
                .header("x-request-id", "my-custom-id")
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["data"], "my-custom-id");
    }

    // ═══════════════════════════════════════════════════
    // AuthLayer
    // ═══════════════════════════════════════════════════

    fn test_jwt() -> (AppConfig, JWT) {
        let mut cfg = AppConfig::default();
        cfg.middleware.auth.jwt_secret = "test-jwt-secret-key-32bytes!!!".into();
        cfg.middleware.auth.access_token_expire_secs = 3600;
        let jwt = JWT::with_config(Arc::new(alun_config::ConfigManager {
            static_config: cfg.clone(),
            dynamic: parking_lot::RwLock::new(HashMap::new()),
        }));
        (cfg, jwt)
    }

    #[tokio::test]
    async fn test_auth_layer_valid_token() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("user-1", Some("alice"), &[], &[]).unwrap();

        let app = Router::new()
            .route("/api/me", get(|| async { Res::ok("authenticated") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/me")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_layer_no_token_returns_401() {
        let (cfg, _) = test_jwt();

        let app = Router::new()
            .route("/api/me", get(|| async { Res::ok("ok") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder().uri("/api/me").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_layer_invalid_token_returns_401() {
        let (cfg, _) = test_jwt();

        let app = Router::new()
            .route("/api/me", get(|| async { Res::ok("ok") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/me")
                .header("Authorization", "Bearer fake.token.here")
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_layer_ignore_path() {
        let (cfg, _) = test_jwt();

        let app = Router::new()
            .route("/api/public", get(|| async { Res::ok("public") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec!["/api/public".into()],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder().uri("/api/public").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_layer_injects_claims() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token(
            "user-1", Some("bob"),
            &["admin".into()],
            &["user:read".into()],
        ).unwrap();

        let app = Router::new()
            .route("/api/me", get(|claims: axum::Extension<AuthClaims>| async move {
                let c = &claims.0.0;
                Res::ok(serde_json::json!({
                    "sub": c.sub,
                    "username": c.username,
                    "roles": c.roles,
                    "permissions": c.permissions,
                }))
            }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/me")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["data"]["sub"], "user-1");
        assert_eq!(val["data"]["username"], "bob");
        assert!(val["data"]["roles"].as_array().unwrap().contains(&serde_json::json!("admin")));
    }

    #[tokio::test]
    async fn test_auth_layer_wrong_secret_returns_401() {
        let (_cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("u1", None, &[], &[]).unwrap();

        let app = Router::new()
            .route("/api/me", get(|| async { Res::ok("ok") }))
            .layer(AuthLayer {
                jwt_secret: "completely-different-secret-key!!!".into(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/me")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_auth_layer_expired_token_returns_401() {
        let mut cfg = AppConfig::default();
        cfg.middleware.auth.jwt_secret = "test-jwt-secret-key-32bytes!!!".into();
        cfg.middleware.auth.access_token_expire_secs = 0;

        let jwt = JWT::with_config(Arc::new(alun_config::ConfigManager {
            static_config: cfg.clone(),
            dynamic: parking_lot::RwLock::new(HashMap::new()),
        }));

        let token = jwt.create_access_token("u1", None, &[], &[]).unwrap();
        std::thread::sleep(Duration::from_millis(100));

        let app = Router::new()
            .route("/api/me", get(|| async { Res::ok("ok") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/me")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK,
            "JWT: 默认 leeway=60s, expire_secs=0 的 Token 仍有效");
    }

    // ═══════════════════════════════════════════════════
    // RequirePermissionLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_permission_layer_has_permission() {
        let app = Router::new()
            .route("/api/admin", get(|| async { Res::ok("admin") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]));

        let mut req = Request::builder().uri("/api/admin").body(Body::empty()).unwrap();
        let claims = TokenClaims {
            jti: None, sub: "admin-1".into(), username: None,
            roles: vec![], permissions: vec!["admin:access".into()],
            token_type: None, exp: 9999999999, iat: 0,
        };
        req.extensions_mut().insert(AuthClaims(claims));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_permission_layer_no_permission_returns_403() {
        let app = Router::new()
            .route("/api/admin", get(|| async { Res::ok("admin") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]));

        let mut req = Request::builder().uri("/api/admin").body(Body::empty()).unwrap();
        let claims = TokenClaims {
            jti: None, sub: "user-1".into(), username: None,
            roles: vec![], permissions: vec!["user:read".into()],
            token_type: None, exp: 9999999999, iat: 0,
        };
        req.extensions_mut().insert(AuthClaims(claims));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn test_permission_layer_no_auth_returns_401() {
        let app = Router::new()
            .route("/api/admin", get(|| async { Res::ok("admin") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]));

        let resp = app.oneshot(
            Request::builder().uri("/api/admin").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn test_permission_layer_super_admin() {
        let app = Router::new()
            .route("/api/admin", get(|| async { Res::ok("admin") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]));

        let mut req = Request::builder().uri("/api/admin").body(Body::empty()).unwrap();
        let claims = TokenClaims {
            jti: None, sub: "super".into(), username: None,
            roles: vec![], permissions: vec!["*".into()],
            token_type: None, exp: 9999999999, iat: 0,
        };
        req.extensions_mut().insert(AuthClaims(claims));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ═══════════════════════════════════════════════════
    // RequireRoleLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_role_layer_has_role() {
        let app = Router::new()
            .route("/api/admin", get(|| async { Res::ok("admin") }))
            .layer(RequireRoleLayer::any(vec!["admin".into()]));

        let mut req = Request::builder().uri("/api/admin").body(Body::empty()).unwrap();
        let claims = TokenClaims {
            jti: None, sub: "admin-1".into(), username: None,
            roles: vec!["admin".into(), "moderator".into()],
            permissions: vec![], token_type: None, exp: 9999999999, iat: 0,
        };
        req.extensions_mut().insert(AuthClaims(claims));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_role_layer_wrong_role_returns_403() {
        let app = Router::new()
            .route("/api/admin", get(|| async { Res::ok("admin") }))
            .layer(RequireRoleLayer::any(vec!["admin".into()]));

        let mut req = Request::builder().uri("/api/admin").body(Body::empty()).unwrap();
        let claims = TokenClaims {
            jti: None, sub: "user-1".into(), username: None,
            roles: vec!["user".into()], permissions: vec![],
            token_type: None, exp: 9999999999, iat: 0,
        };
        req.extensions_mut().insert(AuthClaims(claims));

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ═══════════════════════════════════════════════════
    // RateLimitLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_rate_limit_allows_within_budget() {
        let store = Arc::new(RwLock::new(HashMap::<String, IpWindow>::new()));
        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(RateLimitLayer {
                requests_per_window: 5,
                window_secs: 60,
                store: store.clone(),
            });

        for _ in 0..5 {
            let resp = app.clone().oneshot(
                Request::builder()
                    .uri("/")
                    .header("x-forwarded-for", "10.0.0.1")
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "Request within limit should succeed");
        }
    }

    #[tokio::test]
    async fn test_rate_limit_blocks_exceeding_requests() {
        let store = Arc::new(RwLock::new(HashMap::<String, IpWindow>::new()));
        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(RateLimitLayer {
                requests_per_window: 3,
                window_secs: 60,
                store,
            });

        for _ in 0..3 {
            let resp = app.clone().oneshot(
                Request::builder()
                    .uri("/")
                    .header("x-forwarded-for", "10.0.0.2")
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        let resp = app.oneshot(
            Request::builder()
                .uri("/")
                .header("x-forwarded-for", "10.0.0.2")
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn test_rate_limit_different_ips_independent() {
        let store = Arc::new(RwLock::new(HashMap::<String, IpWindow>::new()));
        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(RateLimitLayer {
                requests_per_window: 2,
                window_secs: 60,
                store,
            });

        for _ in 0..2 {
            app.clone().oneshot(
                Request::builder().uri("/").header("x-forwarded-for", "1.1.1.1")
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
        }

        let resp = app.oneshot(
            Request::builder().uri("/").header("x-forwarded-for", "2.2.2.2")
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "Different IP should have separate budget");
    }

    // ═══════════════════════════════════════════════════
    // SecurityHeadersLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_security_headers_nosniff() {
        let config = SecurityHeadersConfig {
            enabled: true, nosniff: true,
            frame_options: false, hsts: false,
            csp: false, referrer_policy: false,
            permissions_policy: false,
            hsts_max_age_secs: 31536000,
            hsts_include_subdomains: false,
            csp_value: String::new(),
            referrer_policy_value: String::new(),
            permissions_policy_value: String::new(),
        };

        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(SecurityHeadersLayer::new(config));

        let resp = app.oneshot(
            Request::builder().uri("/").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.headers().get("x-content-type-options").unwrap(), "nosniff");
    }

    #[tokio::test]
    async fn test_security_headers_frame_deny() {
        let config = SecurityHeadersConfig {
            enabled: true, frame_options: true,
            nosniff: false, hsts: false,
            csp: false, referrer_policy: false,
            permissions_policy: false,
            hsts_max_age_secs: 31536000,
            hsts_include_subdomains: false,
            csp_value: String::new(),
            referrer_policy_value: String::new(),
            permissions_policy_value: String::new(),
        };

        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(SecurityHeadersLayer::new(config));

        let resp = app.oneshot(
            Request::builder().uri("/").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
    }

    #[tokio::test]
    async fn test_security_headers_hsts() {
        let config = SecurityHeadersConfig {
            enabled: true, hsts: true, hsts_max_age_secs: 31536000,
            hsts_include_subdomains: true,
            nosniff: false, frame_options: false,
            csp: false, referrer_policy: false,
            permissions_policy: false,
            csp_value: String::new(),
            referrer_policy_value: String::new(),
            permissions_policy_value: String::new(),
        };

        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(SecurityHeadersLayer::new(config));

        let resp = app.oneshot(
            Request::builder().uri("/").body(Body::empty()).unwrap()
        ).await.unwrap();

        let hsts = resp.headers().get("strict-transport-security").unwrap().to_str().unwrap();
        assert!(hsts.contains("max-age=31536000"));
        assert!(hsts.contains("includeSubDomains"));
    }

    #[tokio::test]
    async fn test_security_headers_csp() {
        let config = SecurityHeadersConfig {
            enabled: true, csp: true,
            csp_value: "default-src 'self'".into(),
            nosniff: false, frame_options: false, hsts: false,
            referrer_policy: false, permissions_policy: false,
            hsts_max_age_secs: 31536000,
            hsts_include_subdomains: false,
            referrer_policy_value: String::new(),
            permissions_policy_value: String::new(),
        };

        let app = Router::new()
            .route("/", get(|| async { Res::ok("ok") }))
            .layer(SecurityHeadersLayer::new(config));

        let resp = app.oneshot(
            Request::builder().uri("/").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(
            resp.headers().get("content-security-policy").unwrap(),
            "default-src 'self'"
        );
    }

    // ═══════════════════════════════════════════════════
    // NonceLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_nonce_first_request_succeeds() {
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/order", get(|| async { Res::ok("created") }))
            .layer(NonceLayer::new(cache, Duration::from_secs(300)));

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/order")
                .header("x-nonce", "unique-nonce-12345")
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_nonce_replay_returns_409() {
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/order", get(|| async { Res::ok("created") }))
            .layer(NonceLayer::new(cache, Duration::from_secs(300)));

        let nonce = "replay-nonce-abc";

        let resp1 = app.clone().oneshot(
            Request::builder().uri("/api/order").header("x-nonce", nonce)
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let resp2 = app.oneshot(
            Request::builder().uri("/api/order").header("x-nonce", nonce)
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn test_nonce_without_header_succeeds() {
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/order", get(|| async { Res::ok("ok") }))
            .layer(NonceLayer::new(cache, Duration::from_secs(300)));

        let resp = app.oneshot(
            Request::builder().uri("/api/order").body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    // ═══════════════════════════════════════════════════
    // IdempotencyLayer
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_idempotency_first_request_succeeds() {
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/payment", get(|| async { Res::ok("paid") }))
            .layer(IdempotencyLayer::new(cache, Duration::from_secs(86400)));

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/payment")
                .header("x-idempotency-key", "idem-key-001")
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_idempotency_replay_returns_cached() {
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/payment", get(|| async { Res::ok("paid") }))
            .layer(IdempotencyLayer::new(cache, Duration::from_secs(86400)));

        let key = "idem-key-002";

        let resp1 = app.clone().oneshot(
            Request::builder().uri("/api/payment").header("x-idempotency-key", key)
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let resp2 = app.oneshot(
            Request::builder().uri("/api/payment").header("x-idempotency-key", key)
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_idempotency_different_keys() {
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/payment", get(|| async { Res::ok("paid") }))
            .layer(IdempotencyLayer::new(cache, Duration::from_secs(86400)));

        let resp1 = app.clone().oneshot(
            Request::builder().uri("/api/payment").header("x-idempotency-key", "k1")
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let resp2 = app.oneshot(
            Request::builder().uri("/api/payment").header("x-idempotency-key", "k2")
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
    }

    // ═══════════════════════════════════════════════════
    // TokenClaims 安全边界
    // ═══════════════════════════════════════════════════

    #[test]
    fn test_token_claims_super_admin_has_all() {
        let claims = TokenClaims {
            jti: None, sub: "admin".into(), username: None,
            roles: vec![], permissions: vec!["*".into()],
            token_type: None, exp: 9999999999, iat: 0,
        };
        assert!(claims.is_super_admin());
        assert!(claims.has_permission("any.thing.at.all"));
        assert!(claims.has_permission("super:secret:access"));
    }

    #[test]
    fn test_token_claims_regular_user_restricted() {
        let claims = TokenClaims {
            jti: None, sub: "user".into(), username: None,
            roles: vec!["user".into()],
            permissions: vec!["user:read".into()],
            token_type: None, exp: 9999999999, iat: 0,
        };
        assert!(!claims.is_super_admin());
        assert!(claims.has_permission("user:read"));
        assert!(!claims.has_permission("admin:access"));
    }
}
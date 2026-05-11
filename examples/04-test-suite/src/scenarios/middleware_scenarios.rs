//! 中间件集成场景测试
//!
//! 模拟完整的中间件链在实际请求中的行为

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
    };
    use alun_web::jwt::JWT;

    fn test_jwt() -> (AppConfig, JWT) {
        let mut cfg = AppConfig::default();
        cfg.middleware.auth.jwt_secret = "scenario-test-secret-key-bytes!!!".into();
        cfg.middleware.auth.access_token_expire_secs = 3600;
        let jwt = JWT::with_config(Arc::new(alun_config::ConfigManager {
            static_config: cfg.clone(),
            dynamic: parking_lot::RwLock::new(HashMap::new()),
        }));
        (cfg, jwt)
    }

    // ═══════════════════════════════════════════════════
    // 场景 1：完整认证 + 权限链
    // AuthLayer → RequirePermissionLayer → Handler
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_full_auth_permission_chain() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token(
            "user-1", Some("alice"),
            &["admin".into()],
            &["admin:access".into()],
        ).unwrap();

        let app = Router::new()
            .route("/api/admin/stats", get(|| async { Res::ok("stats") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/admin/stats")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(val["data"], "stats");
    }

    #[tokio::test]
    async fn test_auth_permission_chain_no_permission() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("user-1", Some("bob"), &[], &[]).unwrap();

        let app = Router::new()
            .route("/api/admin/stats", get(|| async { Res::ok("stats") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/admin/stats")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ═══════════════════════════════════════════════════
    // 场景 2：认证 + 角色 + 权限多层链
    // AuthLayer → RequireRoleLayer → RequirePermissionLayer → Handler
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_auth_role_permission_chain() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token(
            "admin-1", Some("admin"),
            &["admin".into()],
            &["admin:access".into()],
        ).unwrap();

        let app = Router::new()
            .route("/api/admin/settings", get(|| async { Res::ok("settings") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]))
            .layer(RequireRoleLayer::any(vec!["admin".into()]))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/admin/settings")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_auth_role_chain_wrong_role() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token(
            "user-1", Some("jane"),
            &["user".into()],
            &["admin:access".into()],
        ).unwrap();

        let app = Router::new()
            .route("/api/admin/settings", get(|| async { Res::ok("settings") }))
            .layer(RequirePermissionLayer::any(vec!["admin:access".into()]))
            .layer(RequireRoleLayer::any(vec!["admin".into()]))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            });

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/admin/settings")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    // ═══════════════════════════════════════════════════
    // 场景 3：完整安全链（RequestId + SecurityHeaders + Auth）
    // RequestIdLayer → SecurityHeadersLayer → AuthLayer → Handler
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_security_chain_request_id_and_headers() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("u1", None, &[], &[]).unwrap();

        let sec_config = SecurityHeadersConfig {
            enabled: true, nosniff: true, frame_options: true,
            hsts: false, csp: false, referrer_policy: false,
            permissions_policy: false,
            hsts_max_age_secs: 31536000, hsts_include_subdomains: false,
            csp_value: String::new(), referrer_policy_value: String::new(),
            permissions_policy_value: String::new(),
        };

        let app = Router::new()
            .route("/api/secure", get(|| async { Res::ok("secure") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            })
            .layer(SecurityHeadersLayer::new(sec_config))
            .layer(RequestIdLayer);

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/secure")
                .header("Authorization", format!("Bearer {}", token))
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(resp.headers().get("x-content-type-options").unwrap(), "nosniff");
        assert_eq!(resp.headers().get("x-frame-options").unwrap(), "DENY");
    }

    // ═══════════════════════════════════════════════════
    // 场景 4：限流 + 认证链
    // RateLimitLayer → AuthLayer → Handler
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_rate_limit_auth_chain() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("u1", None, &[], &[]).unwrap();
        let store = Arc::new(RwLock::new(HashMap::<String, IpWindow>::new()));

        let app = Router::new()
            .route("/api/data", get(|| async { Res::ok("data") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            })
            .layer(RateLimitLayer {
                requests_per_window: 2,
                window_secs: 60,
                store,
            });

        for _ in 0..2 {
            let resp = app.clone().oneshot(
                Request::builder()
                    .uri("/api/data")
                    .header("Authorization", format!("Bearer {}", token))
                    .header("x-forwarded-for", "10.0.0.10")
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }

        let resp = app.oneshot(
            Request::builder()
                .uri("/api/data")
                .header("Authorization", format!("Bearer {}", token))
                .header("x-forwarded-for", "10.0.0.10")
                .body(Body::empty()).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    // ═══════════════════════════════════════════════════
    // 场景 5：Nonce + Auth 防重放链
    // NonceLayer → AuthLayer → Handler
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_nonce_auth_chain_replay_prevention() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("u1", None, &[], &[]).unwrap();
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/transfer", get(|| async { Res::ok("transferred") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            })
            .layer(NonceLayer::new(cache, Duration::from_secs(300)));

        let nonce = "transfer-nonce-001";

        let resp1 = app.clone().oneshot(
            Request::builder()
                .uri("/api/transfer")
                .header("Authorization", format!("Bearer {}", token))
                .header("x-nonce", nonce)
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);

        let resp2 = app.oneshot(
            Request::builder()
                .uri("/api/transfer")
                .header("Authorization", format!("Bearer {}", token))
                .header("x-nonce", nonce)
                .body(Body::empty()).unwrap()
        ).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::CONFLICT);
    }

    // ═══════════════════════════════════════════════════
    // 场景 6：Idempotency + Auth 幂等链
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_idempotency_auth_chain() {
        let (cfg, jwt) = test_jwt();
        let token = jwt.create_access_token("u1", None, &[], &[]).unwrap();
        let local = LocalCache::new(100, 0);
        let cache = Arc::new(SharedCache::Local(local));

        let app = Router::new()
            .route("/api/pay", get(|| async { Res::ok("paid") }))
            .layer(AuthLayer {
                jwt_secret: cfg.middleware.auth.jwt_secret.clone(),
                ignore_paths: vec![],
                cache: None,
            })
            .layer(IdempotencyLayer::new(cache, Duration::from_secs(86400)));

        let key = "pay-key-001";
        for _ in 0..3 {
            let resp = app.clone().oneshot(
                Request::builder()
                    .uri("/api/pay")
                    .header("Authorization", format!("Bearer {}", token))
                    .header("x-idempotency-key", key)
                    .body(Body::empty()).unwrap()
            ).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "幂等请求都应成功");
        }
    }
}
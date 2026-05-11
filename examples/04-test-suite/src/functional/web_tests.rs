//! alun-web 功能测试
//!
//! 覆盖：Router、JWT、TokenClaims、ValidatedJson、App 构建器

#[cfg(test)]
mod tests {
    use alun_web::router::AlunRouter;
    use alun_web::middleware::TokenClaims;
    use alun_web::jwt::JWT;
    use alun_web::App;
    use alun_config::{AppConfig, ConfigManager};
    use alun_core::api::Res;
    use axum::Router;
    use axum::http::{StatusCode, Request};
    use axum::body::Body;
    use tower::ServiceExt;
    use std::sync::Arc;

    // ──── AlunRouter ─────────────────────────────────

    #[test]
    fn test_router_new() {
        let router = AlunRouter::new();
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_default() {
        let router = AlunRouter::default();
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_add_get() {
        let mut router = AlunRouter::new();
        router.add_get("/hello", || async { "Hello" });
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_add_post() {
        let mut router = AlunRouter::new();
        router.add_post("/submit", || async { "OK" });
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_add_put() {
        let mut router = AlunRouter::new();
        router.add_put("/update", || async { "Updated" });
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_add_delete() {
        let mut router = AlunRouter::new();
        router.add_delete("/remove", || async { "Deleted" });
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_add_route_custom() {
        let mut router = AlunRouter::new();
        router.add_route("PATCH", "/patch", || async { "Patched" });
        let _r: Router = router.into_axum();
    }

    #[test]
    fn test_router_merge() {
        let mut main = AlunRouter::new();
        main.add_get("/root", || async { "root" });

        let mut sub = AlunRouter::new();
        sub.add_get("/sub-path", || async { "sub" });

        main.merge("/api", sub);
        let _r: Router = main.into_axum();
    }

    #[test]
    fn test_router_multiple_routes() {
        let mut router = AlunRouter::new();
        router.add_get("/users", || async { "users" });
        router.add_post("/users", || async { "create" });
        router.add_put("/users/{id}", || async { "update" });
        router.add_delete("/users/{id}", || async { "delete" });

        let _r: Router = router.into_axum();
    }

    // ──── Router → HTTP 请求 ─────────────────────────

    #[tokio::test]
    async fn test_router_get_200() {
        let mut router = AlunRouter::new();
        router.add_get("/ping", || async { Res::ok("pong") });

        let app: Router = router.into_axum();
        let resp = app
            .oneshot(Request::builder().uri("/ping").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_router_not_found() {
        let mut router = AlunRouter::new();
        router.add_get("/exists", || async { "ok" });

        let app: Router = router.into_axum();
        let resp = app
            .oneshot(Request::builder().uri("/nonexistent").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ──── TokenClaims ────────────────────────────────

    #[test]
    fn test_token_claims_user_id() {
        let claims = TokenClaims {
            jti: Some("jti-1".into()),
            sub: "user_001".into(),
            username: Some("alice".into()),
            roles: vec!["admin".into()],
            permissions: vec!["user:read".into(), "user:write".into()],
            token_type: None,
            exp: 9999999999,
            iat: 1700000000,
        };
        assert_eq!(claims.user_id(), "user_001");
    }

    #[test]
    fn test_token_claims_has_role() {
        let claims = TokenClaims {
            jti: None,
            sub: "u1".into(),
            username: None,
            roles: vec!["admin".into(), "moderator".into()],
            permissions: vec![],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.has_role("admin"));
        assert!(claims.has_role("moderator"));
        assert!(!claims.has_role("user"));
    }

    #[test]
    fn test_token_claims_has_any_role() {
        let claims = TokenClaims {
            jti: None,
            sub: "u1".into(),
            username: None,
            roles: vec!["viewer".into()],
            permissions: vec![],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.has_any_role(&["admin", "viewer"]));
        assert!(!claims.has_any_role(&["admin", "editor"]));
    }

    #[test]
    fn test_token_claims_has_all_roles() {
        let claims = TokenClaims {
            jti: None,
            sub: "u1".into(),
            username: None,
            roles: vec!["a".into(), "b".into(), "c".into()],
            permissions: vec![],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.has_all_roles(&["a", "b"]));
        assert!(!claims.has_all_roles(&["a", "d"]));
    }

    #[test]
    fn test_token_claims_has_permission() {
        let claims = TokenClaims {
            jti: None,
            sub: "u1".into(),
            username: None,
            roles: vec![],
            permissions: vec!["user:read".into(), "user:write".into()],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.has_permission("user:read"));
        assert!(!claims.has_permission("admin:access"));
    }

    #[test]
    fn test_token_claims_has_any_permission() {
        let claims = TokenClaims {
            jti: None,
            sub: "u1".into(),
            username: None,
            roles: vec![],
            permissions: vec!["read".into()],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.has_any_permission(&["write", "read"]));
        assert!(!claims.has_any_permission(&["write", "delete"]));
    }

    #[test]
    fn test_token_claims_is_super_admin() {
        let claims = TokenClaims {
            jti: None,
            sub: "admin".into(),
            username: None,
            roles: vec![],
            permissions: vec!["*".into()],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.is_super_admin());

        let claims2 = TokenClaims {
            jti: None,
            sub: "admin2".into(),
            username: None,
            roles: vec![],
            permissions: vec!["*:*:*".into()],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims2.is_super_admin());

        let claims3 = TokenClaims {
            jti: None,
            sub: "normal".into(),
            username: None,
            roles: vec![],
            permissions: vec!["user:read".into()],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(!claims3.is_super_admin());
    }

    #[test]
    fn test_token_claims_super_admin_has_all() {
        let claims = TokenClaims {
            jti: None,
            sub: "admin".into(),
            username: None,
            roles: vec![],
            permissions: vec!["*".into()],
            token_type: None,
            exp: 9999999999,
            iat: 0,
        };
        assert!(claims.has_permission("anything"));
        assert!(claims.has_any_permission(&["anything", "else"]));
    }

    // ──── JWT ────────────────────────────────────────

    fn jwt_test_config() -> Arc<ConfigManager> {
        let mut cfg = AppConfig::default();
        cfg.middleware.auth.jwt_secret = "test-secret-key-32bytes-long!!".into();
        cfg.middleware.auth.access_token_expire_secs = 3600;
        cfg.middleware.auth.refresh_token_expire_secs = 86400;
        Arc::new(ConfigManager {
            static_config: cfg,
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        })
    }

    #[test]
    fn test_jwt_create_access_token() {
        let jwt = JWT::with_config(jwt_test_config());
        let token = jwt.create_access_token(
            "user-1",
            Some("alice"),
            &["admin".into()],
            &["user:read".into()],
        );
        assert!(token.is_ok());
        let token_str = token.unwrap();
        assert!(!token_str.is_empty());
        assert!(token_str.split('.').count() == 3);
    }

    #[test]
    fn test_jwt_create_refresh_token() {
        let jwt = JWT::with_config(jwt_test_config());
        let token = jwt.create_refresh_token("user-1");
        assert!(token.is_ok());
    }

    #[test]
    fn test_jwt_create_and_validate() {
        let jwt = JWT::with_config(jwt_test_config());
        let token_str = jwt.create_access_token(
            "user-1",
            Some("bob"),
            &["user".into()],
            &["read".into()],
        ).unwrap();

        let claims = jwt.validate(&token_str);
        assert!(claims.is_ok());
        let c = claims.unwrap();
        assert_eq!(c.sub, "user-1");
        assert_eq!(c.username, Some("bob".into()));
        assert!(c.has_role("user"));
        assert!(c.has_permission("read"));
    }

    #[test]
    fn test_jwt_validate_invalid() {
        let jwt = JWT::with_config(jwt_test_config());
        assert!(jwt.validate("invalid.token.here").is_err());
    }

    #[test]
    fn test_jwt_validate_wrong_secret() {
        let jwt1 = JWT::with_config(jwt_test_config());
        let token = jwt1.create_access_token("u1", None, &[], &[]).unwrap();

        let mut cfg2 = AppConfig::default();
        cfg2.middleware.auth.jwt_secret = "different-secret-key-here!!!!".into();
        let cm2 = Arc::new(ConfigManager {
            static_config: cfg2,
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        });
        let jwt2 = JWT::with_config(cm2);

        assert!(jwt2.validate(&token).is_err());
    }

    #[test]
    fn test_jwt_with_roles_and_permissions() {
        let jwt = JWT::with_config(jwt_test_config());
        let token = jwt.create_access_token(
            "user-1",
            Some("charlie"),
            &["admin".into(), "editor".into()],
            &["user:read".into(), "user:write".into(), "admin:access".into()],
        ).unwrap();

        let claims = jwt.validate(&token).unwrap();
        assert!(claims.has_role("admin"));
        assert!(claims.has_role("editor"));
        assert!(!claims.has_role("viewer"));
        assert!(claims.has_permission("user:write"));
        assert!(claims.has_permission("admin:access"));
        assert!(!claims.has_permission("super:access"));
    }

    // ──── App 构建器 ─────────────────────────────────

    #[test]
    fn test_app_with_config() {
        let cfg = AppConfig::default();
        let app = App::with_config(cfg);
        assert!(app.is_ok());
    }

    #[test]
    fn test_app_with_config_manager() {
        let cm = jwt_test_config();
        let app = App::with_config_manager(cm);
        assert!(app.is_ok());
    }

    #[test]
    fn test_app_register_routes() {
        let cfg = AppConfig::default();
        let _app = App::with_config(cfg)
            .unwrap()
            .get("/test", || async { Res::ok("it works") })
            .post("/test", || async { Res::ok("created") })
            .put("/test/{id}", || async { Res::ok("updated") })
            .delete("/test/{id}", || async { Res::ok("deleted") });
    }

    #[test]
    fn test_app_route_method() {
        let cfg = AppConfig::default();
        let _app = App::with_config(cfg)
            .unwrap()
            .route("PATCH", "/api/patch", || async { Res::ok("patched") });
    }

    #[test]
    fn test_app_group() {
        let cfg = AppConfig::default();
        App::with_config(cfg)
            .unwrap()
            .group("/api/v1", |app| {
                app.get("/users", || async { Res::ok("list") })
            });
    }

    #[test]
    fn test_app_with_role() {
        let cfg = AppConfig::default();
        App::with_config(cfg)
            .unwrap()
            .with_role("GET", "/admin/dashboard", || async { Res::ok("admin") }, "admin");
    }

    #[test]
    fn test_app_with_permission() {
        let cfg = AppConfig::default();
        App::with_config(cfg)
            .unwrap()
            .with_permission("POST", "/admin/users", || async { Res::ok("created") }, "admin:write");
    }

    #[test]
    fn test_app_on_startup() {
        let cfg = AppConfig::default();
        let _app = App::with_config(cfg)
            .unwrap()
            .on_startup(|| async {});
    }

    #[test]
    fn test_app_default() {
        let _app = App::default();
        // Default should have no config manager
    }

    // ──── Router with Res response ───────────────────

    #[tokio::test]
    async fn test_router_res_json_response() {
        use axum::body::to_bytes;

        let mut router = AlunRouter::new();
        router.add_get("/api/status", || async {
            Res::ok(serde_json::json!({"status": "ok"}))
        });

        let app: Router = router.into_axum();
        let resp = app
            .oneshot(Request::builder().uri("/api/status").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["status"], "ok");
    }

    #[tokio::test]
    async fn test_router_multiple_methods_same_path() {
        let mut router = AlunRouter::new();
        router.add_get("/items", || async { Res::ok("get") });
        router.add_post("/items", || async { Res::ok("post") });

        let app: Router = router.into_axum();

        let get_resp = app.clone()
            .oneshot(Request::builder().uri("/items").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(get_resp.status(), StatusCode::OK);

        let post_resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/items")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap()
            )
            .await
            .unwrap();
        assert_eq!(post_resp.status(), StatusCode::OK);
    }
}
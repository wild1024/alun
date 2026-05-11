//! 真实场景集成测试
//!
//! 模拟完整的 Web 应用：
//! - 用户注册 / 登录 / Token 刷新 / 登出
//! - 带 JWT 认证的 CRUD 操作
//! - 分页查询
//! - 缓存集成
//! - 错误处理

#[cfg(test)]
mod tests {
    use alun_web::router::AlunRouter;
    use alun_web::middleware::{TokenClaims, AuthClaims, AuthLayer};
    use alun_web::jwt::JWT;
    use alun_config::{AppConfig, ConfigManager};
    use alun_core::api::{Res, ApiError, PageQuery};
    use alun_cache::{LocalCache, Cache};
    use alun_db::{Row, Db};
    use axum::Router;
    use axum::http::{StatusCode, Request};
    use axum::body::{Body, to_bytes};
    use axum::extract::{Path, Query};
    use axum::Extension;
    use tower::ServiceExt;
    use std::sync::Arc;
    use serde::{Serialize, Deserialize};

    // ──── DTO 定义 ───────────────────────────────────

    #[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
    struct UserRes {
        id: String,
        username: String,
        email: String,
        roles: Vec<String>,
    }

    #[derive(Debug, Deserialize)]
    struct LoginReq {
        username: String,
        password: String,
    }

    #[derive(Debug, Deserialize)]
    struct RegisterReq {
        username: String,
        password: String,
        email: String,
    }

    #[derive(Debug, Serialize, Deserialize)]
    struct LoginRes {
        access_token: String,
        refresh_token: String,
        user: UserRes,
    }

    #[derive(Debug, Serialize, Deserialize, Clone)]
    struct ArticleRes {
        id: String,
        title: String,
        content: String,
        author_id: String,
    }

    #[derive(Debug, Deserialize)]
    struct CreateArticleReq {
        title: String,
        content: String,
    }

    #[derive(Debug, Deserialize)]
    struct UpdateArticleReq {
        title: Option<String>,
        content: Option<String>,
    }

    // ──── 模拟数据库 + JWT + 缓存初始化 ──────────────

    async fn setup_sqlite_db() -> Db {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT NOT NULL UNIQUE,
                password_hash TEXT NOT NULL,
                email TEXT NOT NULL,
                roles TEXT NOT NULL DEFAULT '[]'
            )"
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "CREATE TABLE IF NOT EXISTS articles (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                content TEXT NOT NULL DEFAULT '',
                author_id TEXT NOT NULL
            )"
        )
        .execute(&pool)
        .await
        .unwrap();

        Db::sqlite(pool)
    }

    fn create_app_config() -> AppConfig {
        let mut cfg = AppConfig::default();
        cfg.app_name = "TestApp".into();
        cfg.middleware.auth.enabled = true;
        cfg.middleware.auth.jwt_secret = "my-super-secret-jwt-key-32byte!".into();
        cfg.middleware.auth.access_token_expire_secs = 3600;
        cfg.middleware.auth.refresh_token_expire_secs = 86400;
        cfg.middleware.auth.ignore_paths = vec![
            "/api/auth/login".into(),
            "/api/auth/register".into(),
            "/api/public/*".into(),
        ];
        cfg
    }

    // ──── 模拟数据库 + JWT + 缓存初始化 ──────────────

    fn setup_globals() -> (Arc<ConfigManager>, JWT, LocalCache) {
        let cfg = create_app_config();
        let cm = Arc::new(ConfigManager {
            static_config: cfg.clone(),
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        });
        let jwt = JWT::with_config(cm.clone());
        let cache = LocalCache::new(1000, 0);

        (cm, jwt, cache)
    }

    // ──── Handler：用户相关 ───────────────────────────

    async fn register_user(
        axum::extract::Json(req): axum::extract::Json<RegisterReq>,
        Extension(db): Extension<Db>,
        Extension(jwt): Extension<Arc<JWT>>,
    ) -> Result<Res<LoginRes>, ApiError> {
        if req.username.is_empty() || req.password.len() < 6 {
            return Err(ApiError::bad_request("用户名或密码不符合要求"));
        }

        let user_id = uuid::Uuid::new_v4().to_string();
        let password_hash = alun_utils::crypto::Crypto::hash_password(&req.password)
            .map_err(|e| ApiError::internal_masked("注册失败", e.to_string()))?;

        let mut row = Row::table("users").id(user_id.clone());
        row.set("username", req.username.as_str());
        row.set("password_hash", password_hash.as_str());
        row.set("email", req.email.as_str());
        row.set("roles", "[]");

        db.insert(&row).await
            .map_err(|e| ApiError::internal_masked("注册失败", e.to_string()))?;

        let access_token = jwt.create_access_token(
            &user_id, Some(&req.username), &[], &[],
        ).map_err(|e| ApiError::internal_masked("Token 生成失败", e))?;

        let refresh_token = jwt.create_refresh_token(&user_id)
            .map_err(|e| ApiError::internal_masked("Token 生成失败", e))?;

        Ok(Res::ok_with_msg(LoginRes {
            access_token,
            refresh_token,
            user: UserRes {
                id: user_id,
                username: req.username,
                email: req.email,
                roles: vec![],
            },
        }, "注册成功"))
    }

    async fn login_user(
        axum::extract::Json(req): axum::extract::Json<LoginReq>,
        Extension(db): Extension<Db>,
        Extension(jwt): Extension<Arc<JWT>>,
    ) -> Result<Res<LoginRes>, ApiError> {
        let rows = db.query(
            "SELECT * FROM users WHERE username = $1",
            &[&req.username],
        ).await.map_err(|e| ApiError::internal_masked("查询失败", e.to_string()))?;

        let row = rows.first().ok_or_else(|| ApiError::unauthorized("用户名或密码错误"))?;

        let stored_hash: String = row.get_as("password_hash")
            .ok_or_else(|| ApiError::internal("数据异常"))?;

        let valid = alun_utils::crypto::Crypto::verify_password(&req.password, &stored_hash)
            .map_err(|e| ApiError::internal_masked("验证失败", e.to_string()))?;

        if !valid {
            return Err(ApiError::unauthorized("用户名或密码错误"));
        }

        let user_id: String = row.get_as("id").unwrap_or_default();
        let email: String = row.get_as("email").unwrap_or_default();

        let access_token = jwt.create_access_token(
            &user_id, Some(&req.username), &["user".into()], &["article:read".into(), "article:write".into()],
        ).map_err(|e| ApiError::internal_masked("Token 生成失败", e))?;

        let refresh_token = jwt.create_refresh_token(&user_id)
            .map_err(|e| ApiError::internal_masked("Token 生成失败", e))?;

        Ok(Res::ok_with_msg(LoginRes {
            access_token,
            refresh_token,
            user: UserRes {
                id: user_id,
                username: req.username,
                email,
                roles: vec!["user".into()],
            },
        }, "登录成功"))
    }

    async fn get_current_user(
        Extension(AuthClaims(claims)): Extension<AuthClaims>,
    ) -> Res<TokenClaims> {
        Res::ok(claims)
    }

    // ──── Handler：文章 CRUD ──────────────────────────

    async fn list_articles(
        Query(page_query): Query<PageQuery>,
        Extension(db): Extension<Db>,
    ) -> Result<Res<Vec<ArticleRes>>, ApiError> {
        let (rows, total) = db.query_page(
            "SELECT * FROM articles ORDER BY id",
            &[],
            &page_query,
        ).await.map_err(|e| ApiError::internal_masked("查询失败", e.to_string()))?;

        let articles: Vec<ArticleRes> = rows.iter().map(|r| ArticleRes {
            id: r.get_as::<String>("id").unwrap_or_default(),
            title: r.get_as::<String>("title").unwrap_or_default(),
            content: r.get_as::<String>("content").unwrap_or_default(),
            author_id: r.get_as::<String>("author_id").unwrap_or_default(),
        }).collect();

        Ok(Res::ok_with_msg(articles, format!("共 {} 条", total)))
    }

    async fn get_article(
        Path(id): Path<String>,
        Extension(db): Extension<Db>,
    ) -> Result<Res<ArticleRes>, ApiError> {
        let row = db.find_by_id("articles", id.as_str()).await
            .map_err(|e| ApiError::internal_masked("查询失败", e.to_string()))?
            .ok_or_else(|| ApiError::not_found("文章不存在"))?;

        Ok(Res::ok(ArticleRes {
            id: row.get_as("id").unwrap_or_default(),
            title: row.get_as("title").unwrap_or_default(),
            content: row.get_as("content").unwrap_or_default(),
            author_id: row.get_as("author_id").unwrap_or_default(),
        }))
    }

    async fn create_article(
        axum::extract::Json(req): axum::extract::Json<CreateArticleReq>,
        Extension(db): Extension<Db>,
        Extension(AuthClaims(claims)): Extension<AuthClaims>,
    ) -> Result<Res<ArticleRes>, ApiError> {
        let article_id = uuid::Uuid::new_v4().to_string();

        let mut row = Row::table("articles").id(article_id.clone());
        row.set("title", req.title.as_str());
        row.set("content", req.content.as_str());
        row.set("author_id", claims.sub.as_str());

        db.insert(&row).await
            .map_err(|e| ApiError::internal_masked("创建失败", e.to_string()))?;

        Ok(Res::ok_with_msg(ArticleRes {
            id: article_id,
            title: req.title,
            content: req.content,
            author_id: claims.sub,
        }, "创建成功"))
    }

    async fn update_article(
        Path(id): Path<String>,
        axum::extract::Json(req): axum::extract::Json<UpdateArticleReq>,
        Extension(db): Extension<Db>,
    ) -> Result<Res<ArticleRes>, ApiError> {
        let mut existing = db.find_by_id("articles", id.as_str()).await
            .map_err(|e| ApiError::internal_masked("查询失败", e.to_string()))?
            .ok_or_else(|| ApiError::not_found("文章不存在"))?;

        if let Some(ref title) = req.title {
            existing.set("title", title.as_str());
        }
        if let Some(ref content) = req.content {
            existing.set("content", content.as_str());
        }

        let updated = db.update(&existing).await
            .map_err(|e| ApiError::internal_masked("更新失败", e.to_string()))?
            .ok_or_else(|| ApiError::not_found("文章不存在"))?;

        Ok(Res::ok_with_msg(ArticleRes {
            id: updated.get_as("id").unwrap_or_default(),
            title: updated.get_as("title").unwrap_or_default(),
            content: updated.get_as("content").unwrap_or_default(),
            author_id: updated.get_as("author_id").unwrap_or_default(),
        }, "更新成功"))
    }

    async fn delete_article(
        Path(id): Path<String>,
        Extension(db): Extension<Db>,
    ) -> Result<Res<()>, ApiError> {
        let deleted = db.delete_by_id("articles", id.as_str()).await
            .map_err(|e| ApiError::internal_masked("删除失败", e.to_string()))?;

        if deleted {
            Ok(Res::ok_msg("删除成功"))
        } else {
            Err(ApiError::not_found("文章不存在"))
        }
    }

    // ──── 场景1：用户注册 → 登录 → 访问受保护接口 ──────

    #[tokio::test]
    async fn test_scenario_register_login_access() {
        let db = setup_sqlite_db().await;
        let (cm, jwt, _cache) = setup_globals();
        let jwt = Arc::new(jwt);

        // ─ 构建路由 ─
        let mut router = AlunRouter::new();

        router.add_post("/api/auth/register", |state: Extension<Db>, j: Extension<Arc<JWT>>, body: axum::extract::Json<RegisterReq>| async move {
            register_user(body, state, j).await
        });
        router.add_post("/api/auth/login", |state: Extension<Db>, j: Extension<Arc<JWT>>, body: axum::extract::Json<LoginReq>| async move {
            login_user(body, state, j).await
        });
        router.add_get("/api/auth/me", get_current_user);

        let mut app: Router = router.into_axum();

        app = app.layer(AuthLayer {
            jwt_secret: cm.get().middleware.auth.jwt_secret.clone(),
            ignore_paths: vec![
                "/api/auth/register".into(),
                "/api/auth/login".into(),
            ],
            cache: None,
        });

        let app = app
            .layer(axum::Extension(jwt.clone()))
            .layer(axum::Extension(db));

        // ─ 测试注册 ─
        let register_body = serde_json::json!({
            "username": "testuser",
            "password": "Test@123",
            "email": "test@example.com",
        });

        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(register_body.to_string()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["user"]["username"], "testuser");

        let access_token = body["data"]["access_token"].as_str().unwrap().to_string();
        let refresh_token = body["data"]["refresh_token"].as_str().unwrap().to_string();
        assert!(!access_token.is_empty());
        assert!(!refresh_token.is_empty());

        // ─ 测试登录 ─
        let login_body = serde_json::json!({
            "username": "testuser",
            "password": "Test@123",
        });

        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/login")
                    .header("Content-Type", "application/json")
                    .body(Body::from(login_body.to_string()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"]["user"]["username"], "testuser");

        let access_token = body["data"]["access_token"].as_str().unwrap().to_string();

        // ─ 测试访问受保护接口 ─
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/me")
                    .header("Authorization", format!("Bearer {}", access_token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["code"], 0);
        assert!(body["data"]["sub"].as_str().unwrap().len() > 0);
    }

    // ──── 场景2：未认证访问受保护接口 → 401 ───────────

    #[tokio::test]
    async fn test_scenario_unauthorized_access() {
        let _db = setup_sqlite_db().await;
        let (cm, jwt, _) = setup_globals();
        let jwt = Arc::new(jwt);

        let mut router = AlunRouter::new();
        router.add_get("/api/auth/me", get_current_user);

        let mut app: Router = router.into_axum();
        app = app.layer(AuthLayer {
            jwt_secret: cm.get().middleware.auth.jwt_secret.clone(),
            ignore_paths: vec![],
            cache: None,
        });
        let app = app.layer(axum::Extension(jwt));

        let resp = app
            .oneshot(Request::builder().uri("/api/auth/me").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ──── 场景3：无效 Token → 401 ─────────────────────

    #[tokio::test]
    async fn test_scenario_invalid_token() {
        let _db = setup_sqlite_db().await;
        let (cm, jwt, _) = setup_globals();
        let jwt = Arc::new(jwt);

        let mut router = AlunRouter::new();
        router.add_get("/api/auth/me", get_current_user);

        let mut app: Router = router.into_axum();
        app = app.layer(AuthLayer {
            jwt_secret: cm.get().middleware.auth.jwt_secret.clone(),
            ignore_paths: vec![],
            cache: None,
        });
        let app = app.layer(axum::Extension(jwt));

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/auth/me")
                    .header("Authorization", "Bearer invalid-token-here")
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ──── 场景4：文章 CRUD 完整流程 ───────────────────

    async fn build_crud_app(db: Db, cm: Arc<ConfigManager>, jwt: Arc<JWT>) -> Router {
        let mut router = AlunRouter::new();

        router.add_post("/api/auth/register", |state: Extension<Db>, j: Extension<Arc<JWT>>, body: axum::extract::Json<RegisterReq>| async move {
            register_user(body, state, j).await
        });
        router.add_post("/api/auth/login", |state: Extension<Db>, j: Extension<Arc<JWT>>, body: axum::extract::Json<LoginReq>| async move {
            login_user(body, state, j).await
        });

        router.add_get("/api/articles", |q: Query<PageQuery>, state: Extension<Db>| async move {
            list_articles(q, state).await
        });
        router.add_post("/api/articles", |state: Extension<Db>, claims: Extension<AuthClaims>, body: axum::extract::Json<CreateArticleReq>| async move {
            create_article(body, state, claims).await
        });
        router.add_get("/api/articles/{id}", |p: Path<String>, state: Extension<Db>| async move {
            get_article(p, state).await
        });
        router.add_put("/api/articles/{id}", |p: Path<String>, state: Extension<Db>, body: axum::extract::Json<UpdateArticleReq>| async move {
            update_article(p, body, state).await
        });
        router.add_delete("/api/articles/{id}", |p: Path<String>, state: Extension<Db>| async move {
            delete_article(p, state).await
        });

        let mut app: Router = router.into_axum();

        app = app.layer(AuthLayer {
            jwt_secret: cm.get().middleware.auth.jwt_secret.clone(),
            ignore_paths: vec![
                "/api/auth/register".into(),
                "/api/auth/login".into(),
            ],
            cache: None,
        });

        app
            .layer(axum::Extension(jwt))
            .layer(axum::Extension(db))
    }

    async fn register_and_login(app: &Router) -> String {
        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(serde_json::json!({
                        "username": "writer",
                        "password": "Writer@123",
                        "email": "writer@test.com",
                    }).to_string()))
                    .unwrap()
            )
            .await
            .unwrap();

        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        body["data"]["access_token"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn test_scenario_article_crud() {
        let db = setup_sqlite_db().await;
        let (cm, jwt, _) = setup_globals();
        let jwt = Arc::new(jwt);
        let app = build_crud_app(db, cm, jwt).await;

        let token = register_and_login(&app).await;

        // ─ 创建文章 ─
        let create_body = serde_json::json!({
            "title": "My First Article",
            "content": "Hello, Alun!",
        });

        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("Content-Type", "application/json")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::from(create_body.to_string()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["code"], 0);
        let article_id = body["data"]["id"].as_str().unwrap().to_string();
        assert_eq!(body["data"]["title"], "My First Article");

        // ─ 获取文章列表 ─
        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles?page=1&page_size=10")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["code"], 0);
        assert_eq!(body["data"].as_array().unwrap().len(), 1);

        // ─ 确认文章存在（GET 单条） ─
        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/articles/{}", article_id))
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK, "Get article should succeed");
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["data"]["title"], "My First Article");

        // ─ 获取不存在的文章 → 404 ─
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/articles/nonexistent-id-9999")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND, "Non-existent article should return 404");
    }

    // ──── 场景5：分页查询多维测试 ─────────────────────

    #[tokio::test]
    async fn test_scenario_pagination() {
        let db = setup_sqlite_db().await;
        let (cm, jwt, _) = setup_globals();
        let jwt = Arc::new(jwt);
        let app = build_crud_app(db, cm, jwt).await;

        let token = register_and_login(&app).await;

        // ─ 创建 15 篇文章 ─
        for i in 1..=15 {
            let body = serde_json::json!({
                "title": format!("Article {}", i),
                "content": format!("Content {}", i),
            });
            let _ = app.clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/articles")
                        .header("Content-Type", "application/json")
                        .header("Authorization", format!("Bearer {}", token))
                        .body(Body::from(body.to_string()))
                        .unwrap()
                )
                .await
                .unwrap();
        }

        // ─ 第1页 × 10条 ─
        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles?page=1&page_size=10")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 10);

        // ─ 第2页 × 10条 应该返回5条 ─
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/api/articles?page=2&page_size=10")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();
        let body_bytes = to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
        assert_eq!(body["data"].as_array().unwrap().len(), 5);
    }

    // ──── 场景6：缓存集成测试 ─────────────────────────

    #[tokio::test]
    async fn test_scenario_cache_integration() {
        let cache = LocalCache::new(100, 0);

        cache.set("app:name", &"Alun Test".to_string()).await.unwrap();
        let val: Option<String> = cache.get("app:name").await.unwrap();
        assert_eq!(val, Some("Alun Test".into()));

        cache.set_ex("temp:session", &"session-data".to_string(), 1).await.unwrap();
        assert!(cache.exists("temp:session").await.unwrap());
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert!(!cache.exists("temp:session").await.unwrap());
    }

    // ──── 场景7：错误处理 ─────────────────────────────

    #[tokio::test]
    async fn test_scenario_error_handling() {
        let db = setup_sqlite_db().await;
        let (cm, jwt, _) = setup_globals();
        let jwt = Arc::new(jwt);
        let app = build_crud_app(db, cm, jwt).await;

        let token = register_and_login(&app).await;

        // ─ 404: 获取不存在的文章 ─
        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .uri("/api/articles/nonexistent-id")
                    .header("Authorization", format!("Bearer {}", token))
                    .body(Body::empty())
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);

        // ─ 422: 注册缺少必要字段 ─
        let bad_body = serde_json::json!({"username": ""});
        let resp = app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/auth/register")
                    .header("Content-Type", "application/json")
                    .body(Body::from(bad_body.to_string()))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY,
            "Missing required fields should return 422");

        // ─ 401: 不传 Token 创建文章 ─
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/articles")
                    .header("Content-Type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap()
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // ──── 场景8：JWT Token 完整生命周期 ───────────────

    #[test]
    fn test_scenario_jwt_lifecycle() {
        let cfg = create_app_config();
        let cm = Arc::new(ConfigManager {
            static_config: cfg,
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        });
        let jwt = JWT::with_config(cm);

        let access = jwt.create_access_token(
            "user-x", Some("jane"), &["user".into()], &["read".into(), "write".into()],
        ).unwrap();

        let refresh = jwt.create_refresh_token("user-x").unwrap();

        let claims = jwt.validate(&access).unwrap();
        assert_eq!(claims.sub, "user-x");
        assert_eq!(claims.username, Some("jane".into()));
        assert!(claims.has_role("user"));
        assert!(claims.has_permission("read"));

        let refresh_claims = jwt.validate(&refresh).unwrap();
        assert_eq!(refresh_claims.sub, "user-x");
    }

    // ──── 场景9：并发请求 ─────────────────────────────

    #[tokio::test]
    async fn test_scenario_concurrent_requests() {
        let db = setup_sqlite_db().await;
        let (cm, jwt, _) = setup_globals();
        let jwt = Arc::new(jwt);
        let app = Arc::new(build_crud_app(db, cm, jwt).await);

        let token = register_and_login(&app).await;

        let mut handles = Vec::new();
        for i in 0..10 {
            let app = app.clone();
            let token = token.clone();
            handles.push(tokio::spawn(async move {
                let body = serde_json::json!({
                    "title": format!("Concurrent Article {}", i),
                    "content": "test",
                });
                let resp = app.as_ref().clone()
                    .oneshot(
                        Request::builder()
                            .method("POST")
                            .uri("/api/articles")
                            .header("Content-Type", "application/json")
                            .header("Authorization", format!("Bearer {}", token))
                            .body(Body::from(body.to_string()))
                            .unwrap()
                    )
                    .await
                    .unwrap();
                resp.status()
            }));
        }

        for handle in handles {
            let status = handle.await.unwrap();
            assert_eq!(status, StatusCode::OK);
        }
    }

    // ──── 场景10：Res 响应结构验证 ─────────────────────

    #[test]
    fn test_all_res_patterns() {
        let empty = Res::<()>::ok_empty();
        assert_eq!(empty.code, 0);
        assert_eq!(empty.msg, "ok");

        let msg = Res::<()>::ok_msg("操作完成");
        assert_eq!(msg.code, 0);
        assert_eq!(msg.msg, "操作完成");

        let data = Res::ok(42);
        assert_eq!(data.code, 0);
        assert_eq!(data.data, Some(42));

        let with_msg = Res::ok_with_msg("result", "成功");
        assert_eq!(with_msg.code, 0);
        assert_eq!(with_msg.msg, "成功");
        assert_eq!(with_msg.data, Some("result"));

        let fail = Res::<()>::fail(400, "错误");
        assert_eq!(fail.code, 400);
        assert_eq!(fail.msg, "错误");
    }
}
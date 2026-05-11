//! Alun 框架压力测试
//!
//! 覆盖：高并发请求、大载荷、持续负载、
//! 限流器高压、数据库并发、缓存压力

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::http::{StatusCode, Request};
    use axum::body::{Body, to_bytes};
    use tower::ServiceExt;
    use parking_lot::RwLock;
    use std::collections::HashMap;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use alun_core::api::Res;
    use alun_config::SecurityHeadersConfig;
    use alun_cache::{LocalCache, SharedCache, Cache};
    use alun_web::middleware::{
        RateLimitLayer, IpWindow, SecurityHeadersLayer,
        RequestIdLayer, IdempotencyLayer,
    };

    // ═══════════════════════════════════════════════════
    // 并发请求压力
    // ═══════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread")]
    async fn test_concurrent_requests_stress() {
        let app = Arc::new(Router::new()
            .route("/", axum::routing::get(|| async { Res::ok("ok") }))
            .layer(RequestIdLayer));

        let concurrency = 100;
        let start = Instant::now();
        let mut handles = Vec::new();

        for _i in 0..concurrency {
            let app = app.clone();
            handles.push(tokio::spawn(async move {
                let resp = app.as_ref().clone()
                    .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                resp.status()
            }));
        }

        let mut ok_count = 0;
        for h in handles {
            let status = h.await.unwrap();
            if status == StatusCode::OK { ok_count += 1; }
        }

        let elapsed = start.elapsed();
        assert_eq!(ok_count, concurrency,
            "并发压力: 所有请求应成功 ({}), 耗时 {:?}", ok_count, elapsed);
        assert!(elapsed < Duration::from_secs(30),
            "并发压力: 100 并发应在 30s 内完成, 实际 {:?}", elapsed);
    }

    // ═══════════════════════════════════════════════════
    // 大载荷请求
    // ═══════════════════════════════════════════════════

    #[tokio::test]
    async fn test_large_payload_handling() {
        let app = Router::new()
            .route("/api/upload", axum::routing::post(|axum::extract::Json(body): axum::extract::Json<serde_json::Value>| async move {
                let len = body.to_string().len();
                Res::ok(serde_json::json!({"size": len}))
            }));

        let large_data = serde_json::json!({
            "items": (0..1000).map(|i| serde_json::json!({
                "id": i,
                "name": format!("item_{}", i),
                "description": "A".repeat(200),
            })).collect::<Vec<_>>()
        });

        let _body_size = large_data.to_string().len();
        let resp = app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upload")
                .header("Content-Type", "application/json")
                .body(Body::from(large_data.to_string())).unwrap()
        ).await.unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), 10 * 1024 * 1024).await.unwrap();
        let val: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let returned_size = val["data"]["size"].as_u64().unwrap();
        assert!(returned_size > 100_000,
            "大载荷: 应正确处理 >100KB 的请求体 (size={})", returned_size);
    }

    #[tokio::test]
    async fn test_very_large_payload_rejected() {
        let app = Router::new()
            .route("/api/upload", axum::routing::post(|axum::extract::Json(body): axum::extract::Json<serde_json::Value>| async move {
                Res::ok(serde_json::json!({"size": body.to_string().len()}))
            }));

        let very_large = serde_json::json!({
            "data": "A".repeat(5_000_000)
        });

        let resp = app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/upload")
                .header("Content-Type", "application/json")
                .body(Body::from(very_large.to_string())).unwrap()
        ).await.unwrap();

        assert!(!resp.status().is_server_error(),
            "超大载荷: 不应导致 5xx, 实际 {}", resp.status().as_u16());
    }

    // ═══════════════════════════════════════════════════
    // 持续负载
    // ═══════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread")]
    async fn test_sustained_load_stress() {
        let app = Arc::new(Router::new()
            .route("/", axum::routing::get(|| async { Res::ok("ok") }))
            .layer(RequestIdLayer));

        let rounds = 10;
        let per_round = 50;
        let start = Instant::now();

        for round in 0..rounds {
            let mut handles = Vec::new();
            for _ in 0..per_round {
                let app = app.clone();
                handles.push(tokio::spawn(async move {
                    app.as_ref().clone()
                        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
                        .await.unwrap()
                        .status()
                }));
            }
            for h in handles {
                assert_eq!(h.await.unwrap(), StatusCode::OK,
                    "持续负载: 第 {} 轮请求失败", round + 1);
            }
        }

        let elapsed = start.elapsed();
        assert!(elapsed < Duration::from_secs(60),
            "持续负载: 500 总请求应在 60s 内完成, 实际 {:?}", elapsed);
    }

    // ═══════════════════════════════════════════════════
    // 限流器高压
    // ═══════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread")]
    async fn test_rate_limiter_under_pressure() {
        let store = Arc::new(RwLock::new(HashMap::<String, IpWindow>::new()));
        let app = Arc::new(Router::new()
            .route("/", axum::routing::get(|| async { Res::ok("ok") }))
            .layer(RateLimitLayer {
                requests_per_window: 20,
                window_secs: 60,
                store,
            }));

        let total_requests = 60;
        let start = Instant::now();
        let mut handles = Vec::new();

        for _ in 0..total_requests {
            let app = app.clone();
            handles.push(tokio::spawn(async move {
                app.as_ref().clone()
                    .oneshot(Request::builder()
                        .uri("/")
                        .header("x-forwarded-for", "stress-test-ip")
                        .body(Body::empty()).unwrap())
                    .await.unwrap()
                    .status()
            }));
        }

        let mut ok = 0;
        let mut limited = 0;
        for h in handles {
            match h.await.unwrap() {
                StatusCode::OK => ok += 1,
                StatusCode::TOO_MANY_REQUESTS => limited += 1,
                _ => {}
            }
        }

        let elapsed = start.elapsed();
        assert_eq!(ok, 20, "限流高压: 应恰好通过 20 次 (配额)");
        assert_eq!(limited, 40, "限流高压: 应限流 40 次 (超出配额)");
        assert!(elapsed < Duration::from_secs(20),
            "限流高压: 60 并发应在 20s 内完成, 实际 {:?}", elapsed);
    }

    // ═══════════════════════════════════════════════════
    // 数据库并发压力
    // ═══════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread")]
    async fn test_database_concurrent_reads() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query("CREATE TABLE stress_test (id TEXT PRIMARY KEY, val INTEGER)")
            .execute(&pool).await.unwrap();
        for i in 0..100 {
            sqlx::query("INSERT INTO stress_test VALUES ($1, $2)")
                .bind(format!("id_{}", i))
                .bind(i)
                .execute(&pool).await.unwrap();
        }

        let db = std::sync::Arc::new(alun_db::Db::sqlite(pool));
        let app = Arc::new(Router::new()
            .route("/api/data", axum::routing::get({
                let db = db.clone();
                move || {
                    let db = db.clone();
                    async move {
                        let count = db.count("SELECT COUNT(*) FROM stress_test", &[]).await.unwrap();
                        Res::ok(count)
                    }
                }
            })));

        let concurrency = 30;
        let start = Instant::now();
        let mut handles = Vec::new();

        for _ in 0..concurrency {
            let app = app.clone();
            handles.push(tokio::spawn(async move {
                let resp = app.as_ref().clone()
                    .oneshot(Request::builder().uri("/api/data").body(Body::empty()).unwrap())
                    .await.unwrap();
                resp.status()
            }));
        }

        let mut ok = 0;
        for h in handles {
            if h.await.unwrap() == StatusCode::OK { ok += 1; }
        }

        let elapsed = start.elapsed();
        assert_eq!(ok, concurrency,
            "DB并发: 30 并发读应全部成功, 耗时 {:?}", elapsed);
        assert!(elapsed < Duration::from_secs(10),
            "DB并发: 应在 10s 内完成, 实际 {:?}", elapsed);
    }

    // ═══════════════════════════════════════════════════
    // 缓存压力
    // ═══════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cache_concurrent_writes() {
        let cache = Arc::new(LocalCache::new(5000, 0));

        let concurrency = 50;
        let items_per_task = 100;
        let start = Instant::now();
        let mut handles = Vec::new();

        for t in 0..concurrency {
            let c = cache.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..items_per_task {
                    let key = format!("task_{}_item_{}", t, i);
                    c.set(&key, &("v".to_string())).await.unwrap();
                }
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let elapsed = start.elapsed();
        let len = cache.len();
        assert_eq!(len, concurrency * items_per_task as usize,
            "缓存写入: 所有 {} 条目应写入, 实际 {} 条", concurrency * items_per_task as usize, len);
        assert!(elapsed < Duration::from_secs(10),
            "缓存写入: 5000 条应在 10s 内完成, 实际 {:?}", elapsed);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cache_concurrent_read_write() {
        let cache = Arc::new(LocalCache::new(5000, 0));

        for i in 0..500 {
            cache.set(&format!("key_{}", i), &format!("value_{}", i)).await.unwrap();
        }

        let mut handles = Vec::new();
        for t in 0..20 {
            let c = cache.clone();
            handles.push(tokio::spawn(async move {
                for i in 0..500 {
                    if t % 2 == 0 {
                        let _ = c.incr("counter", 1).await;
                    } else {
                        let _key = format!("key_{}", i);
                        let _: Option<String> = c.get(&_key).await.unwrap();
                    }
                }
            }));
        }

        for h in handles { h.await.unwrap(); }

        let count = cache.incr("counter", 0).await.unwrap();
        assert_eq!(count as usize, 10 * 500,
            "缓存读写: 计数器应为 5000, 实际 {}", count);
    }

    // ═══════════════════════════════════════════════════
    // 中间件链压力
    // ═══════════════════════════════════════════════════

    #[tokio::test(flavor = "multi_thread")]
    async fn test_full_middleware_chain_stress() {
        let local = LocalCache::new(500, 0);
        let cache = Arc::new(SharedCache::Local(local));
        let store = Arc::new(RwLock::new(HashMap::<String, IpWindow>::new()));

        let sec_cfg = SecurityHeadersConfig {
            enabled: true, nosniff: true, frame_options: true,
            hsts: true, csp: true, referrer_policy: true,
            permissions_policy: true,
            hsts_max_age_secs: 31536000, hsts_include_subdomains: true,
            csp_value: "default-src 'self'".into(),
            referrer_policy_value: "strict-origin".into(),
            permissions_policy_value: "camera=()".into(),
        };

        let app = Arc::new(Router::new()
            .route("/", axum::routing::get(|| async { Res::ok("ok") }))
            .layer(IdempotencyLayer::new(cache.clone(), Duration::from_secs(60)))
            .layer(RateLimitLayer {
                requests_per_window: 200,
                window_secs: 60,
                store,
            })
            .layer(SecurityHeadersLayer::new(sec_cfg))
            .layer(RequestIdLayer));

        let concurrency = 50;
        let start = Instant::now();
        let mut handles = Vec::new();

        for i in 0..concurrency {
            let app = app.clone();
            handles.push(tokio::spawn(async move {
                app.as_ref().clone()
                    .oneshot(Request::builder()
                        .uri("/")
                        .header("x-idempotency-key", format!("mk-{}", i))
                        .body(Body::empty()).unwrap())
                    .await.unwrap()
                    .status()
            }));
        }

        let mut ok = 0;
        for h in handles {
            if h.await.unwrap() == StatusCode::OK { ok += 1; }
        }

        let elapsed = start.elapsed();
        assert_eq!(ok, concurrency,
            "中间件链压力: 50 并发应全部成功, 耗时 {:?}", elapsed);
        assert!(elapsed < Duration::from_secs(15),
            "中间件链压力: 应在 15s 内完成, 实际 {:?}", elapsed);
    }
}
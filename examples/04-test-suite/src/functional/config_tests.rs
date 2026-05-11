//! alun-config 功能测试
//!
//! 覆盖：AppConfig、默认值、多环境、ConfigManager、环境变量覆盖

#[cfg(test)]
mod tests {
    use alun_config::{AppConfig, ConfigManager, ServerConfig, LogConfig,
        DatabaseConfig, CacheConfig, MiddlewareConfig, RouterConfig, NotFoundConfig};
    

    // ──── AppConfig 默认值 ────────────────────────────

    #[test]
    fn test_default_app_name() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.app_name, "Alun");
    }

    #[test]
    fn test_default_profile() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.profile, "dev");
    }

    #[test]
    fn test_default_server_listen() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.server.listen, "8023");
    }

    #[test]
    fn test_default_log_level() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.log.level, "info");
    }

    #[test]
    fn test_default_log_format() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.log.format, "text");
    }

    #[test]
    fn test_default_db_type() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.database.r#type, "postgres");
    }

    #[test]
    fn test_default_db_disabled() {
        let cfg = AppConfig::default();
        assert!(!cfg.database.enabled);
    }

    #[test]
    fn test_default_cache_type() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.cache.r#type, "local");
    }

    #[test]
    fn test_default_cache_capacity() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.cache.max_capacity, 10000);
    }

    #[test]
    fn test_default_cache_ttl() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.cache.default_ttl, 3600);
    }

    #[test]
    fn test_default_auth_expire() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.middleware.auth.access_token_expire_secs, 7200);
        assert_eq!(cfg.middleware.auth.refresh_token_expire_secs, 604800);
    }

    #[test]
    fn test_default_rate_limit() {
        let cfg = AppConfig::default();
        assert!(!cfg.middleware.rate_limit.enabled);
        assert_eq!(cfg.middleware.rate_limit.requests_per_window, 100);
        assert_eq!(cfg.middleware.rate_limit.window_secs, 60);
    }

    #[test]
    fn test_default_security_headers() {
        let cfg = AppConfig::default();
        assert!(cfg.middleware.security_headers.enabled);
        assert!(cfg.middleware.security_headers.nosniff);
    }

    #[test]
    fn test_default_upload_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.upload.path, "uploads");
        assert_eq!(cfg.upload.max_size_mb, 10);
    }

    #[test]
    fn test_default_download_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.download.path, "downloads");
    }

    #[test]
    fn test_default_template_config() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.template.path, "templates");
    }

    #[test]
    fn test_default_static_config() {
        let cfg = AppConfig::default();
        assert!(!cfg.static_files.enabled);
        assert_eq!(cfg.static_files.path, "static");
    }

    #[test]
    fn test_default_router_prefix() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.router.prefix, "");
    }

    // ──── ServerConfig ────────────────────────────────

    #[test]
    fn test_server_config_default() {
        let s = ServerConfig::default();
        assert_eq!(s.listen, "8023");
    }

    #[test]
    fn test_server_config_custom() {
        let s = ServerConfig { listen: ":3000".into() };
        assert_eq!(s.listen, ":3000");
    }

    // ──── LogConfig ───────────────────────────────────

    #[test]
    fn test_log_config_default() {
        let l = LogConfig::default();
        assert_eq!(l.level, "info");
        assert_eq!(l.format, "text");
        assert!(l.dir.is_none());
        assert_eq!(l.file_prefix, "alun");
    }

    // ──── DatabaseConfig ──────────────────────────────

    #[test]
    fn test_database_config_default() {
        let d = DatabaseConfig::default();
        assert!(!d.enabled);
        assert_eq!(d.r#type, "postgres");
        assert_eq!(d.host, "localhost");
        assert_eq!(d.max_connections, 10);
        assert_eq!(d.min_connections, 2);
        assert_eq!(d.connect_timeout, 10);
        assert!(!d.sql_logging);
        assert_eq!(d.slow_query_ms, 0);
    }

    // ──── CacheConfig ─────────────────────────────────

    #[test]
    fn test_cache_config_default() {
        let c = CacheConfig::default();
        assert_eq!(c.r#type, "local");
        assert_eq!(c.max_capacity, 10000);
        assert_eq!(c.default_ttl, 3600);
    }

    // ──── ConfigManager ───────────────────────────────

    #[test]
    fn test_config_manager_dynamic() {
        let cfg = AppConfig::default();
        let cm = ConfigManager {
            static_config: cfg,
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        };

        assert!(cm.get_dynamic("test_key").is_none());

        cm.set_dynamic("test_key", serde_json::json!("hello"));
        assert_eq!(
            cm.get_dynamic("test_key").unwrap(),
            serde_json::json!("hello")
        );

        cm.remove_dynamic("test_key");
        assert!(cm.get_dynamic("test_key").is_none());
    }

    #[test]
    fn test_config_manager_dynamic_complex_value() {
        let cfg = AppConfig::default();
        let cm = ConfigManager {
            static_config: cfg,
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        };

        let value = serde_json::json!({
            "key": "val",
            "num": 42,
            "arr": [1, 2, 3]
        });
        cm.set_dynamic("complex", value.clone());
        assert_eq!(cm.get_dynamic("complex").unwrap(), value);
    }

    #[test]
    fn test_config_manager_get_static() {
        let cfg = AppConfig::default();
        let cm = ConfigManager {
            static_config: cfg,
            dynamic: parking_lot::RwLock::new(std::collections::HashMap::new()),
        };

        let static_cfg = cm.get();
        assert_eq!(static_cfg.app_name, "Alun");
        assert_eq!(static_cfg.server.listen, "8023");
    }

    // ──── 序列化测试 ──────────────────────────────────

    #[test]
    fn test_app_config_serialization() {
        let cfg = AppConfig::default();
        let toml_str = toml::to_string_pretty(&cfg).unwrap();
        assert!(toml_str.contains("listen = \"8023\""));
        assert!(toml_str.contains("level = \"info\""));
    }

    #[test]
    fn test_app_config_deserialization() {
        let toml_str = r#"
app_name = "TestApp"
profile = "prod"

[server]
listen = "8080"
"#;
        let cfg: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.app_name, "TestApp");
        assert_eq!(cfg.profile, "prod");
        assert_eq!(cfg.server.listen, "8080");
    }

    #[test]
    fn test_partial_toml_uses_defaults() {
        let toml_str = r#"
app_name = "Partial"

[server]
listen = "9999"
"#;
        let cfg: AppConfig = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.app_name, "Partial");
        assert_eq!(cfg.server.listen, "9999");
        assert_eq!(cfg.log.level, "info");
        assert_eq!(cfg.database.r#type, "postgres");
    }

    // ──── 中间件配置 ──────────────────────────────────

    #[test]
    fn test_middleware_default_all_disabled() {
        let mw = MiddlewareConfig::default();
        assert!(!mw.request_id);
        assert!(!mw.request_log);
        assert!(!mw.auth.enabled);
        assert!(!mw.cors.enabled);
        assert!(!mw.compression.enabled);
        assert!(!mw.rate_limit.enabled);
        assert!(mw.security_headers.enabled);
    }

    #[test]
    fn test_auth_middleware_default() {
        let auth = alun_config::AuthMiddlewareConfig::default();
        assert!(!auth.enabled);
        assert!(auth.jwt_secret.is_empty());
        assert!(auth.ignore_paths.is_empty());
    }

    #[test]
    fn test_cors_config_default() {
        let cors = alun_config::CorsConfig::default();
        assert!(!cors.enabled);
        assert!(cors.allow_origins.is_empty());
        assert!(cors.allow_credentials);
    }

    // ──── RouterPrefix ────────────────────────────────

    #[test]
    fn test_router_config_custom_prefix() {
        let rc = RouterConfig { prefix: "/api/v1".into(), not_found: NotFoundConfig { enabled: true, message: "Not Found".into() } };
        assert_eq!(rc.prefix, "/api/v1");
    }

    // ──── 生成默认配置 ────────────────────────────────

    #[test]
    fn test_generate_default_config() {
        let tmp = std::env::temp_dir().join("alun_cfg_test");
        let _ = std::fs::remove_dir_all(&tmp);
        ConfigManager::generate_default(tmp.to_str().unwrap()).unwrap();

        let config_path = tmp.join("config.toml");
        assert!(config_path.exists());

        let content = std::fs::read_to_string(&config_path).unwrap();
        assert!(content.contains("app_name = \"Alun\""));
        assert!(content.contains("listen = \"8023\""));
        assert!(content.contains("level = \"info\""));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    // ──── Profile 合并 ────────────────────────────────

    #[test]
    fn test_detect_profile_default() {
        let profile = alun_config::env::detect_profile();
        assert!(!profile.is_empty());
    }
}
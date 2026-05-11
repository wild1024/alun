//! alun-cache 功能测试
//!
//! 覆盖：LocalCache set/get/del/exists/incr/keys/delete_pattern/stats/TTL

#[cfg(test)]
mod tests {
    use alun_cache::{LocalCache, Cache};
    use std::time::Duration;

    // ──── 基本 set/get ───────────────────────────────

    #[tokio::test]
    async fn test_set_get_string() {
        let c = LocalCache::new(100, 0);
        c.set("key1", &"hello".to_string()).await.unwrap();
        let val: Option<String> = c.get("key1").await.unwrap();
        assert_eq!(val, Some("hello".into()));
    }

    #[tokio::test]
    async fn test_set_get_number() {
        let c = LocalCache::new(100, 0);
        c.set("num", &42).await.unwrap();
        let val: Option<i32> = c.get("num").await.unwrap();
        assert_eq!(val, Some(42));
    }

    #[tokio::test]
    async fn test_get_missing_key() {
        let c = LocalCache::new(100, 0);
        let val: Option<String> = c.get("nonexistent").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_set_overwrite() {
        let c = LocalCache::new(100, 0);
        c.set("key", &"first".to_string()).await.unwrap();
        c.set("key", &"second".to_string()).await.unwrap();
        let val: Option<String> = c.get("key").await.unwrap();
        assert_eq!(val, Some("second".into()));
    }

    // ──── set_ex / TTL ───────────────────────────────

    #[tokio::test]
    async fn test_set_ex_with_ttl() {
        let c = LocalCache::new(100, 0);
        c.set_ex("temp", &"expiring".to_string(), 2).await.unwrap();
        let val: Option<String> = c.get("temp").await.unwrap();
        assert_eq!(val, Some("expiring".into()));
    }

    #[tokio::test]
    async fn test_set_ex_expired() {
        let c = LocalCache::new(100, 0);
        c.set_ex("temp", &"expire_value".to_string(), 1).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        let val: Option<String> = c.get("temp").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_default_ttl() {
        let c = LocalCache::new(100, 1);
        c.set("ttl_key", &"short_lived").await.unwrap();
        let val: Option<String> = c.get("ttl_key").await.unwrap();
        assert_eq!(val, Some("short_lived".into()));

        tokio::time::sleep(Duration::from_secs(2)).await;
        let val: Option<String> = c.get("ttl_key").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_no_ttl_by_default() {
        let c = LocalCache::new(100, 0);
        c.set("permanent", &"forever").await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        let val: Option<String> = c.get("permanent").await.unwrap();
        assert_eq!(val, Some("forever".into()));
    }

    // ──── del ────────────────────────────────────────

    #[tokio::test]
    async fn test_del_existing_key() {
        let c = LocalCache::new(100, 0);
        c.set("del_key", &"value").await.unwrap();
        c.del("del_key").await.unwrap();
        let val: Option<String> = c.get("del_key").await.unwrap();
        assert_eq!(val, None);
    }

    #[tokio::test]
    async fn test_del_nonexistent_key() {
        let c = LocalCache::new(100, 0);
        c.del("not_there").await.unwrap();
    }

    // ──── exists ─────────────────────────────────────

    #[tokio::test]
    async fn test_exists_true() {
        let c = LocalCache::new(100, 0);
        c.set("exists_key", &"val").await.unwrap();
        assert!(c.exists("exists_key").await.unwrap());
    }

    #[tokio::test]
    async fn test_exists_false() {
        let c = LocalCache::new(100, 0);
        assert!(!c.exists("missing").await.unwrap());
    }

    #[tokio::test]
    async fn test_exists_expired() {
        let c = LocalCache::new(100, 0);
        c.set_ex("expired_key", &"val", 1).await.unwrap();
        tokio::time::sleep(Duration::from_secs(2)).await;
        assert!(!c.exists("expired_key").await.unwrap());
    }

    // ──── incr ───────────────────────────────────────

    #[tokio::test]
    async fn test_incr_new_key() {
        let c = LocalCache::new(100, 0);
        assert_eq!(c.incr("counter", 1).await.unwrap(), 1);
    }

    #[tokio::test]
    async fn test_incr_existing_key() {
        let c = LocalCache::new(100, 0);
        c.set("counter", &10).await.unwrap();
        assert_eq!(c.incr("counter", 5).await.unwrap(), 15);
    }

    #[tokio::test]
    async fn test_incr_negative() {
        let c = LocalCache::new(100, 0);
        c.set("counter", &20).await.unwrap();
        assert_eq!(c.incr("counter", -7).await.unwrap(), 13);
    }

    #[tokio::test]
    async fn test_incr_multiple() {
        let c = LocalCache::new(100, 0);
        for i in 1..=10 {
            assert_eq!(c.incr("seq", 1).await.unwrap(), i);
        }
    }

    // ──── keys ───────────────────────────────────────

    #[tokio::test]
    async fn test_keys_exact_pattern() {
        let c = LocalCache::new(100, 0);
        c.set("user:1", &"a").await.unwrap();
        c.set("user:2", &"b").await.unwrap();
        let keys = c.keys("user:*").await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn test_keys_no_match() {
        let c = LocalCache::new(100, 0);
        c.set("user:1", &"a").await.unwrap();
        let keys = c.keys("order:*").await.unwrap();
        assert_eq!(keys.len(), 0);
    }

    #[tokio::test]
    async fn test_keys_wildcard_all() {
        let c = LocalCache::new(100, 0);
        c.set("a", &"1").await.unwrap();
        c.set("b", &"2").await.unwrap();
        let keys = c.keys("*").await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    // ──── delete_pattern ─────────────────────────────

    #[tokio::test]
    async fn test_delete_pattern_single() {
        let c = LocalCache::new(100, 0);
        c.set("session:abc", &"s1").await.unwrap();
        c.set("session:def", &"s2").await.unwrap();
        c.set("keep:key", &"k1").await.unwrap();
        let n = c.delete_pattern("session:*").await.unwrap();
        assert_eq!(n, 2);
        assert!(!c.exists("session:abc").await.unwrap());
        assert!(c.exists("keep:key").await.unwrap());
    }

    #[tokio::test]
    async fn test_delete_pattern_none() {
        let c = LocalCache::new(100, 0);
        let n = c.delete_pattern("nonexistent:*").await.unwrap();
        assert_eq!(n, 0);
    }

    // ──── stats ──────────────────────────────────────

    #[tokio::test]
    async fn test_cache_stats() {
        let c = LocalCache::new(100, 0);
        c.set("s1", &"v1").await.unwrap();
        c.set("s2", &"v2").await.unwrap();
        let _: Option<String> = c.get("s1").await.unwrap();
        let _: Option<String> = c.get("missing").await.unwrap();
        c.del("s2").await.unwrap();

        let stats = c.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 1);
        assert_eq!(stats.sets, 2);
        assert_eq!(stats.deletes, 1);
    }

    // ──── capacity ───────────────────────────────────

    #[tokio::test]
    async fn test_cache_capacity() {
        let c = LocalCache::new(2, 0);
        c.set("k1", &"v1").await.unwrap();
        c.set("k2", &"v2").await.unwrap();
        let result = c.set("k3", &"v3").await;
        assert!(result.is_err());
    }

    // ──── len / is_empty ─────────────────────────────

    #[tokio::test]
    async fn test_len_and_is_empty() {
        let c = LocalCache::new(100, 0);
        assert!(c.is_empty());
        assert_eq!(c.len(), 0);

        c.set("k1", &"v1").await.unwrap();
        assert!(!c.is_empty());
        assert_eq!(c.len(), 1);
    }

    // ──── cleanup ────────────────────────────────────

    #[tokio::test]
    async fn test_cleanup_expired() {
        let c = LocalCache::new(100, 0);
        c.set_ex("exp1", &"v1", 1).await.unwrap();
        c.set_ex("exp2", &"v2", 1).await.unwrap();
        c.set("keep", &"v3").await.unwrap();

        tokio::time::sleep(Duration::from_secs(2)).await;

        let cleaned = c.cleanup_expired();
        assert_eq!(cleaned, 2);
        assert_eq!(c.len(), 1);
    }

    // ──── 布尔值操作 ─────────────────────────────────

    #[tokio::test]
    async fn test_cache_bool_values() {
        let c = LocalCache::new(100, 0);
        c.set("flag_true", &true).await.unwrap();
        c.set("flag_false", &false).await.unwrap();

        let val1: Option<bool> = c.get("flag_true").await.unwrap();
        let val2: Option<bool> = c.get("flag_false").await.unwrap();

        assert_eq!(val1, Some(true));
        assert_eq!(val2, Some(false));
    }

    // ──── 复杂 JSON ─────────────────────────────────

    #[tokio::test]
    async fn test_cache_json_value() {
        let c = LocalCache::new(100, 0);
        let obj = serde_json::json!({
            "name": "alice",
            "age": 30,
            "roles": ["user", "admin"]
        });
        c.set("user", &obj).await.unwrap();
        let val: Option<serde_json::Value> = c.get("user").await.unwrap();
        assert_eq!(val, Some(obj));
    }
}
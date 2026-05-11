# Plugins, Caching & Templates

## Plugin Trait

```rust
use async_trait::async_trait;
use alun_core::{Plugin, Result};

struct MyPlugin;

#[async_trait]
impl Plugin for MyPlugin {
    fn name(&self) -> &str { "my-plugin" }
    async fn start(&self) -> Result<()> { Ok(()) }
    async fn stop(&self) -> Result<()> { Ok(()) }
    fn depends_on(&self) -> &[&str] { &["db"] }  // Ensures DB plugin starts first
}
```

### Plugin Lifecycle

- **start**: Called in topological order (dependencies first). If any plugin fails, startup aborts.
- **stop**: Called in reverse topological order. Failures are logged but do not abort shutdown.
- **depends_on**: Declares ordering. Cyclic deps → `Config` error at startup.

## Built-in Plugins

Enable via config:
```toml
[plugins]
enabled = ["cache", "notification", "async-task", "scheduler"]
```

**CachePlugin**: Auto-creates `SharedCache` (Local/Redis). Access via `cache()`.

**SchedulerPlugin** (cron-based):
```rust
use alun_plugin::scheduler::SchedulerPlugin;

let scheduler = SchedulerPlugin::new(4);  // 4 workers
scheduler.register(
    "cleanup",
    "0 */2 * * *",        // Cron: every 2 hours
    "清理临时文件",
    || Box::pin(async { Ok(()) }),
);
scheduler.trigger("cleanup").await?;  // Manual trigger
let jobs = scheduler.list();           // List jobs
```

**NotificationPlugin** (SMTP):
```rust
use alun_plugin::notification::NotificationPlugin;

let notif = NotificationPlugin::from_config(&config.notification);
notif.send_text("admin@example.com", "Alert", "Disk usage > 90%").await?;
notif.send_html("user@example.com", "验证码", "<h1>Your Code</h1>").await?;
```

**AsyncTaskPlugin** (semaphore-based):
```rust
use alun_plugin::async_task::AsyncTaskPlugin;

let pool = AsyncTaskPlugin::new(4);  // 4 workers
pool.submit(async { heavy_work().await; });
```

## Registering Custom Plugins

```rust
#[alun::plugin]
struct MyCustomPlugin;

#[async_trait]
impl Plugin for MyCustomPlugin {
    fn name(&self) -> &str { "my-custom" }
    async fn start(&self) -> Result<()> {
        tracing::info!("Custom plugin started");
        Ok(())
    }
    async fn stop(&self) -> Result<()> { Ok(()) }
    fn depends_on(&self) -> &[&str] { &["db"] }
}

App::new()?
    .plugin(MyCustomPlugin)
    .scan()
    .start()
    .await
```

---

## Caching

Requires `features = ["cache"]`.

### Global Access

```rust
// Unwrapping (panics if not initialized)
cache().set("key", "value").await?;
cache().set_ex("key", "value", 300).await?;      // 5 min TTL
let val: Option<String> = cache().get("key").await?;
let exists: bool = cache().exists("key").await?;
cache().del("key").await?;

// Counter
let count: i64 = cache().incr("api_calls", 1).await?;

// Pattern matching
let keys: Vec<String> = cache().keys("user:*").await?;
let deleted: u64 = cache().delete_pattern("temp:*").await?;

// Stats
let stats = cache().stats().await?;

// Safe accessor
try_cache()  // Option<&SharedCache>
```

### Standalone Usage

```rust
use alun_cache::{create_cache, Cache, LocalCache};

let local = LocalCache::new(10000, 3600);  // Capacity 10000, default TTL 3600s
local.set("key", "value").await?;

// From config
let cache = alun_cache::create_cache(&cfg().cache, &cfg().redis).await?;
```

### Cache Types

`SharedCache::Local(Arc<LocalCache>)` — in-memory, suitable for single-instance.
`SharedCache::Redis(Arc<RedisCache>)` — distributed via Redis.

---

## Template Engine

Requires `features = ["template"]`. Uses minijinja (Jinja2 syntax).

### Global Access

```rust
let html = render_template("page.html", &json!({"title": "Home", "items": [...]}))
    .map_err(|e| ApiError::internal(e.to_string()))?;

// Safe accessor
try_template()  // Option<&TemplateEngine>
```

### Standalone Usage

```rust
use alun_template::TemplateEngine;

let engine = TemplateEngine::from_dir("templates")?;
let html = engine.render("index.html", &json!({"title": "Alun"}))?;  // From file
let result = engine.render_str("Hello, {{ name }}!", &json!({"name": "World"}))?;  // From string
```

Template files go in `templates/` directory by default.
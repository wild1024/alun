# Configuration System

## Loading Order

1. `config/config.toml` — base config
2. `config/config-{profile}.toml` — profile override (optional)
3. `ALUN_*` environment variables — final override

## Global Access

```rust
// Static config (immutable after load)
let port = cfg().server.listen;
let db_type = &cfg().database.r#type;
let jwt_enabled = cfg().middleware.auth.enabled;

// Dynamic config (runtime read/write)
config().set_dynamic("rate_limit.requests_per_window", serde_json::json!(200));
let limit: Option<i32> = config().get_dynamic("rate_limit.requests_per_window")
    .and_then(|v| v.as_i64().map(|n| n as i32));
config().remove_dynamic("temp.key");

// Custom section
let val = cfg().custom.get("my_app_key")
    .and_then(|v| v.as_str());

// Safe accessors (return Option — no panic)
try_config()  // Option<&Arc<ConfigManager>>
```

## CLI & Environment

```bash
# Generate default config file
cargo run -- gen-config

# Print current resolved config
cargo run -- print-config

# Activate a profile
cargo run -- profile=prod
ALUN_PROFILE=prod cargo run

# Override config items via env vars
ALUN_SERVER_LISTEN=3000 cargo run
ALUN_LOG_LEVEL=debug cargo run
ALUN_DATABASE_HOST=10.0.0.1 cargo run
ALUN_DATABASE_MIGRATION_AUTO_MIGRATE=true cargo run
```

## Startup Hooks

Execute custom initialisation after global resources are ready, before plugins start:

```rust
App::new()?
    .on_startup(|| async {
        // Initialize custom globals here
    })
    .scan()
    .start()
    .await
```

## Testing Pattern

`App::empty()` creates an App without config loading:

```rust
App::empty()
    .get("/", || async { "OK" })
    .serve("8080")
    .await
```

## Plugin Configuration

Enable built-in plugins via the `[plugins]` section:

```toml
[plugins]
enabled = ["cache", "notification", "async-task", "scheduler", "serial"]

[plugins.serial]
backend = "memory"     # memory | redis | custom

[[plugins.serial.rules]]
key = "order"
format = "ORD{YYYY}{MM}{DD}{SEQ:8}"
cycle = "daily"
initial_value = 1
step = "sequential"
```

Each plugin has its own sub-config section (e.g., `[plugins.scheduler]`, `[plugins.serial]`).
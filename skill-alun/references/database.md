# Database (`alun-db`)

Requires `features = ["db"]`. `Db` is a unified facade over PostgreSQL (`$N`), MySQL (`?`), and SQLite (`?`).

## Global Access

```rust
db()                                         // &Db — panics if DB not initialized
try_db()                                     // Option<&Db> — safe accessor
```

## Row Pattern CRUD

The `Row` struct is alun's data carrier — a typed `HashMap` with change tracking:

```rust
use alun::prelude::*;

// Insert
let row = Row::table("users")
    .id(Sid::uuid7())
    .set("name", "张三")
    .set("email", "zhangsan@example.com")
    .set("age", 28);
let inserted: Row = db().insert(&row).await?;

// Query by ID (auto-detects ID type: UUID/i64/String)
let user: Option<Row> = db().find_by_id("users", "u1").await?;

// Raw SQL
let users: Vec<Row> = db().query("SELECT * FROM users WHERE active = $1 ORDER BY id", &["true"]).await?;
let one: Option<Row> = db().query_one("SELECT * FROM users WHERE email = $1", &["a@b.com"]).await?;
let total: u64 = db().count("SELECT COUNT(*) FROM users WHERE active = $1", &["true"]).await?;

// Paginated query
let (rows, total) = db().query_page(
    "SELECT * FROM users ORDER BY created_at DESC",
    &[],
    &PageQuery::new(1, 20),
).await?;

// Update (only fields in `changes` are sent to DB)
let mut row = db().find_by_id("users", "u1").await?.unwrap();
row.set("age", 29);
let updated: Option<Row> = db().update(&row).await?;

// Delete
let deleted: bool = db().delete_by_id("users", "u1").await?;
let count: u64 = db().batch_delete_by_ids("users", &["u1", "u2"]).await?;

// Batch insert
let rows: Vec<Row> = data.iter().map(|u| {
    Row::table("users").id(Sid::uuid7()).set("name", &u.name).set("email", &u.email)
}).collect();
let affected: u64 = db().batch_insert(&rows).await?;

// Batch conditional update
db().batch_update("users", &set_row, "status = $1", &["inactive"]).await?;
```

## Row Field Access

```rust
row.get("name")              // Option<&Value>
row.get_as::<String>("name") // Option<String>
row.get_as::<i64>("age")     // Option<i64>
row.get_id()                 // Option<&Value>
row.has("field")             // bool
row.mark_all_changed()       // Mark all fields as changed
row.clear_changes()          // Clear change tracking (subsequent set() will only contain new fields)
row.detect_id_kind()         // Option<IdKind>
```

For custom primary keys: `.primary_key("pk_name").id(value)`. Composite keys: `.primary_keys(&["key1", "key2"])`.

## Transactions (RAII Guarantee)

Transactions are closure-based — return `Ok(())` commits, `Err` rollbacks. The compiler enforces rollback:

```rust
db().transaction(|tx| async move {
    tx.execute("UPDATE accounts SET balance = balance - 100 WHERE id = $1", &["from"]).await?;
    tx.execute("UPDATE accounts SET balance = balance + 100 WHERE id = $1", &["to"]).await?;

    let balance = tx.query_one("SELECT balance FROM accounts WHERE id = $1", &["from"]).await?;
    if balance.and_then(|r| r.get_as::<i64>("balance")).unwrap_or(0) < 0 {
        tx.set_rollback_only();  // Force rollback even if Ok is returned
    }
    Ok(())  // Commit unless rollback_only is set
}).await?;
```

The closure signature is `FnOnce(ActiveTx) -> Fut` where `Fut` outputs `(ActiveTx, DbResult<T>)`. Always return `Ok(())` from the innermost closure.

## Hooks (CRUD Lifecycle)

Implement the `Hook` trait:

```rust
use alun_db::{Hook, HookChain, TimestampHook};

struct AuditHook;

#[async_trait]
impl Hook for AuditHook {
    async fn after_insert(&self, row: &Row) -> DbResult<()> {
        tracing::info!("审计 - 新增: table={:?}, id={:?}", row.table, row.get_id());
        Ok(())
    }
}

let hook = HookChain::new()
    .add(TimestampHook::new("created_at", "updated_at"))
    .add(AuditHook);
```

## SQL Templates (Jinja2)

```rust
use alun_db::SqlTemplate;

let mut tpl = SqlTemplate::new();
tpl.add("search", r#"
    SELECT * FROM users WHERE 1=1
    {% if name %} AND name LIKE '%{{ name }}%' {% endif %}
    {% if status %} AND status = {{ status }} {% endif %}
    ORDER BY created_at DESC
"#);

let mut params = HashMap::new();
params.insert("name".into(), "张三".into());
let sql = tpl.render("search", &params)?;
let rows = db().query(&sql, &[]).await?;
```

## Migrations

Files go in `migrations/NNN_description.up.sql` and `.down.sql`:

```rust
use alun_db::Migrator;

let migrator = Migrator::new(&pool, "migrations");
migrator.run().await?;       // Run all unapplied *.up.sql
migrator.rollback().await?;  // Rollback the latest migration
```

## Configuration

```toml
[database]
enabled = true
type = "postgres"          # postgres | mysql | sqlite
host = "localhost"
port = 5432
name = "mydb"
user = "app_user"
password = ""
password_encrypted = false  # AES-GCM encrypted password
max_connections = 10
min_connections = 2
sql_logging = false
slow_query_ms = 0           # 0 = disabled

[database.migration]
enabled = false
path = "migrations"
auto_migrate = false
```

## Key Enums

| Enum | Variants |
|------|----------|
| `Dialect` | `Postgres` (`$N`), `Mysql` (`?`), `Sqlite` (`?`) |
| `Isolation` | `ReadUncommitted`, `ReadCommitted` (default), `RepeatableRead`, `Serializable` |
| `IdKind` | `Uuid`, `I64`, `F64`, `String`, `Bool` |
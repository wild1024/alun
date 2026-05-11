# alun

Alun: A fast, simple, and productive Rust web framework - config-driven, plugin-extensible, batteries included.

## Quick Start

```rust
use alun::prelude::*;

#[alun::get("/")]
async fn hello() -> Res<String> {
    Res::ok("Hello, alun!".into())
}

#[tokio::main]
async fn main() {
    App::new().expect("Failed to initialize").scan().start().await.unwrap();
}
```

## Features

- **Config-driven**: Behavior determined by `config.toml`
- **Zero-cost abstraction**: Pure Rust traits + generics
- **Batteries included**: Auth, DB, Cache, Template, Kafka, Task, and more
- **Safety by default**: Security headers, Nonce, Idempotency, XSS protection

## Documentation

See [docs.rs/alun](https://docs.rs/alun) for full documentation.
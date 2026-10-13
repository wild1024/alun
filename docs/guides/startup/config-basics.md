---
id: guides.startup.config-basics
title: 写配置与启动应用
summary: config/config.toml 怎么写、三种启动方式怎么选、启动时到底发生了什么（排查问题按这个顺序看日志）。
tags: [getting-started, config, startup, app]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 写配置与启动应用

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `config-basics` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。


## 3.1 生成默认配置

```bash
mkdir -p config
cargo run -- gen-config      # 生成 config/config.toml（等价写法：generate-config）
cargo run -- print-config    # 打印「当前真正生效」的配置
```

## 3.2 一个够用的最小配置

```toml
# config/config.toml
app_name = "Alun"        # 注意：app_name / profile 是根级标量，没有 [app] 表
profile = "dev"

[server]
listen = "8023"          # 监听地址：端口号或 ip:port

[log]
level = "info"           # trace | debug | info | warn | error
format = "text"          # text（彩色，开发用） | json（结构化，采集用）
dir = "logs"             # 注释掉则只输出到 stdout
```

## 3.3 配置从哪来：加载顺序

1. `config/config.toml` —— 基础配置；
2. `config/config-{profile}.toml` —— profile 覆盖（可选）；
3. `ALUN_*` 环境变量 —— 最终覆盖。

profile 怎么定：命令行 `profile=prod` > 环境变量 `ALUN_PROFILE` > 环境变量 `ALUN_ENV` > 默认 `dev`。

```bash
cargo run -- profile=prod
ALUN_PROFILE=prod cargo run
```

**⚠️ 两个必须知道的坑**（很多「配置没生效」都出在这儿）：

1. **`config.toml` 写错格式会静默回退到全默认配置** —— 解析失败不会报错，你只会觉得「改了没用」。
   所以改完配置第一件事是 `cargo run -- print-config` 看生效值。
2. **profile 文件只能「覆盖成非默认值」，不能把基础配置里的开关关掉**。
   例如基础配置 `[middleware.auth] enabled = true`，profile 里写 `enabled = false` **不会**生效
   （它等于默认值，合并时被跳过）。要按环境关功能，就用环境变量或拆不同的基础配置。

## 3.4 环境变量覆盖（白名单）

只有下面这些 `ALUN_*` 会被读取，其余一律忽略（例如 `ALUN_DATABASE_MIGRATION_AUTO_MIGRATE` **无效**，
迁移开关请写进 `config.toml`）：

```bash
ALUN_SERVER_LISTEN=3000 cargo run
ALUN_LOG_LEVEL=debug cargo run
ALUN_LOG_FORMAT=json cargo run
ALUN_LOG_DIR=logs cargo run
ALUN_DATABASE_HOST=10.0.0.1 cargo run
ALUN_DATABASE_NAME=mydb cargo run
ALUN_DATABASE_USER=app_user cargo run
ALUN_DATABASE_PASSWORD=secret cargo run
ALUN_DATABASE_SSL_MODE=require cargo run
ALUN_REDIS_URL=redis://cache.internal:6379 cargo run
ALUN_CACHE_TYPE=redis cargo run
ALUN_ROUTER_PREFIX=/api/v1 cargo run
ALUN_UPLOAD_PATH=uploads cargo run
ALUN_DOWNLOAD_PATH=downloads cargo run
ALUN_TEMPLATE_PATH=templates cargo run

# JWT 密钥（生产环境必填；FACAI_JWT_SECRET 同义，优先级更高）
ALUN_MIDDLEWARE_AUTH_JWT_SECRET=your-secret cargo run
```

**实战提问：本地开发、测试、生产三套配置怎么管？**

```text
my-project/config/
├── config.toml            # 公共部分（端口、日志、路由前缀、中间件开关）
├── config-dev.toml        # 开发覆盖：SQLite、debug 日志
├── config-test.toml       # 测试覆盖：测试库地址
└── config-prod.toml       # 生产覆盖：PG 地址、json 日志、限流
```

```bash
cargo run -- profile=dev    # 或 ALUN_PROFILE=dev
cargo run -- profile=prod
```

敏感值（数据库密码、JWT 密钥）**不要写进文件**，用环境变量注入。

---

# 4. 第 4 步：启动应用

alun 的启动就一条主线：**读配置 → 初始化全局资源 → 跑启动钩子 → 启动插件 → 装中间件 → 监听端口**。
你要选的只是「用哪种方式起」。

## 方式 A：配置驱动（推荐，默认用法）

```rust,no_run
use alun::prelude::*;

async fn hello() -> Res<&'static str> {
    Res::ok("Hello, Alun!")
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?                      // 从当前目录的 config/ 加载配置
        .get("/", hello)
        .parse_cli()                 // 支持 gen-config / print-config
        .start()                     // 端口取 [server].listen
        .await
}
```

**用在哪**：绝大多数项目。配置改行为，代码只管业务。

## 方式 B：显式指定监听地址

```rust,no_run
use alun::prelude::*;

# async fn hello() -> Res<&'static str> { Res::ok("Hello, Alun!") }

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .get("/", hello)
        .serve("8080")               // 覆盖配置里的 [server].listen
        .await
}
```

**用在哪**：本地换个端口调试；容器里由环境决定端口。

## 方式 C：无配置启动（演示 / 测试 / 最小可复现）

```rust,no_run
use alun::prelude::*;

async fn hello() -> Res<&'static str> {
    Res::ok("OK")
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::with_config(AppConfig::default())?   // 不读 config/ 目录，全用内置默认值
        .get("/", hello)
        .serve("8080")
        .await
}
```

**用在哪**：验证框架行为、写最小复现、给同事发一段能直接跑的代码。
注意此时数据库/缓存等都没配，`db()` 会 panic，关键路径要用 `try_db()`。

## 方式 D：宏注解路由 + `scan()`（默认写法，绝大多数项目用它）

```rust,no_run
use alun::{App, Res};

#[alun::get("/")]
async fn index() -> Res<String> {
    Res::ok("Hello Alun!".into())
}

#[alun::post("/api/user")]
async fn create_user() -> Res<String> {
    Res::ok("created".into())
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?
        .scan()                      // 扫描 #[get] / #[post] 等宏注解，自动注册
        .start()
        .await
}
```

**用在哪**：接口一多，用 `main.rs` 链式注册就不合适了 —— 把 handler 写在各自的业务
`handler.rs` 里，`scan()` 自动发现。
**这是默认路径，日常开发只用这一条就够了**：handler 分散在多少个文件都行，
只有一个 `main.rs` 里加一行 `.scan()`，不需要为任何模块写注册代码。

> **可选通道**：`#[alun::routes]` 标在 `mod` 上，宏生成 `register` 函数，
> 由 `App::module(api::register)` 挂上。它同样零反射、可裁剪、IDE 可跳转，
> 且是库形态分发（插件带自己的接口）的可行通道。
> 普通应用**不必用**；确实需要完全掌控装配顺序时再上，两条通道并存、可混用（`scan()` 先扫宏，`.module()` 再挂显式模块）。

```rust,no_run
use alun::prelude::*;

#[alun::routes]
mod api {
    use alun::prelude::*;

    #[alun::get("/")]
    pub async fn index() -> Res<String> {
        Res::ok("Hello Alun!".into())
    }
}

#[tokio::main]
async fn main() -> alun::Result<()> {
    App::new()?.module(api::register).start().await
}
```

## 方式 E：自定义配置目录

```rust,ignore
// 配置不在 ./config 时（多服务共享一份配置、容器挂载到别的路径）
let app = App::from_config_dir("/etc/myapp")?;
app.scan().start().await?;
```

## 启动时到底做了什么（排查问题按这个顺序看日志）

1. `gen-config` / `print-config` 之类的命令行分支处理；
2. 初始化日志（`[log]` 段生效，之后才有结构化日志）；
3. **初始化全局资源**：配置 → 数据库（建池 + `SELECT 1` 自检 + 可选自动迁移）→ 缓存 → 模板 → 上传/下载目录；
4. **执行 `on_startup` 启动钩子**（全局资源已就绪，插件还没启动）；
5. **启动插件**（按 `depends_on` 拓扑排序；重名报错、循环依赖报错）；
6. 构建 Router 并按配置装配中间件链；
7. 静态文件 / 自定义 404 的 fallback；
8. 绑定端口并监听。

顺序带来的一条实用结论：**凡是「用得上数据库/缓存的东西」，都放在第 4 步之后才初始化** ——
这正是 `on_startup` 存在的理由（详见 [hooks-plugins.md](hooks-plugins.md)）。

**实战提问：启动失败怎么排查？**

| 日志关键字 | 常见原因 |
| ---- | ---- |
| `数据库连接失败: ...` | `[database]` 字段填错、库没起、账号密码不对（日志里的 URL 已脱敏，可对照检查） |
| `数据库迁移失败: ...` | 迁移文件写错、一个文件写了多条语句、改了已发布的迁移（checksum 不一致） |
| `数据库连接 URL 构造失败: ...` | `type` 写错（只认 `postgres` / `postgresql` / `mysql` / `sqlite`），或 `password_encrypted = true` 但没给 `ALUN_SECRET_KEY` |
| `插件循环依赖: [...]` | 两个插件的 `depends_on` 互相指向对方 |
| 进程起来但接口 404 | 路由没注册（忘了 `scan()`）、`[router] prefix` 与请求路径对不上、`[static_files] enabled = true` 时未匹配请求会回退到静态目录 |

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [① 环境与依赖](setup.md) | [③ 数据库与第一个接口](first-endpoint.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

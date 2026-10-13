---
id: architecture.aifei-tribute
title: 致敬 Aifei — 设计理念的 Rust 表达
summary: Alun 承接 Aifei（世界首个 AI Coding 框架）的设计理念，在 Rust 生态的对应实现与提升；含完整设计映射表与官方链接。
tags: [aifei, tribute, architecture, design, ai-coding]
status: active
owner: alun-core-team
source_of_truth: [docs]
depends_on: [architecture.overview]
last_reviewed: 2026-10-06
---

# 致敬 Aifei

> **[Aifei](https://github.com/jfinal/aifei) 是世界首个 AI Coding 框架**（Java / JFinal 作者，专注极简设计 15 年）。
> 官网 [aifei.cn](https://aifei.cn) · 文档 [aifei.cn/doc](https://aifei.cn/doc) · Demo 与生态在 [gitee.com/jfinal/aifei-eco](https://gitee.com/jfinal/aifei-eco)

**Alun 不是 Aifei 的代码翻译，而是 Aifei 设计理念的 Rust 表达。**

Aifei 提出的核心判断是：*AI Coding 时代，框架的首要任务是降低上下文噪音*——框架要优化的不是名义上下文窗口，而是**有效上下文窗口**。它用「Just Service」范式消除 Controller / Render / Repository / Mapper 等非业务结构。

Alun 承接同一判断，在 Rust 生态用**零成本抽象 + 编译期校验**实现同一目标：把代码收敛为「一个接口 = 一个 `async fn` + 一个宏注解」，并让编译器而非约定来保证正确性。

## 为什么两个框架值得对照阅读

| | Aifei（Java） | Alun（Rust） |
|---|---|---|
| 范式名 | Just Service | **Just Config** |
| 一个接口长什么样 | 一个 Service 方法 | 一个 `async fn` + 一个宏注解 |
| 消除掉的层 | Controller / Render / Repository / Mapper | Service / Repository / Mapper（保留可选 `#[controller]` 分组） |
| 数据访问 | `Db` + `Row` + Model 静态糖 | `Db` 门面 + `Row` 动态行 + `#[derive(Model)]` |
| 正确性保障 | 约定 + 运行时 | **编译期**（类型系统 / derive / 方言渲染） |
| 依赖体量 | 内核 3333 行、零第三方依赖 | `alun-core` 零 HTTP / 零数据库依赖 |

两者回答同一个问题：**怎样让大模型把上下文预算花在业务语义上，而不是框架结构上？**

## 设计映射：精华 → Rust 实现

| Aifei 设计精华 | Alun 实现方式 | Rust 带来的增益 |
|--------------|-------------|------|
| 配置即回调启动 | Builder 链式调用 | 编译期校验所有类型，回调变同步构建 |
| 多级拦截器分层 | tower Layer 栈 | 类型安全 + 零运行时开销 + 标准生态 |
| 方法级参数注入 | axum `FromRequest` | 类型安全 Extractor，编译期校验 |
| 全局资源单例 | `std::sync::OnceLock` | 无需 State 注入，一行 `db()` / `cfg()` 即可访问 |
| Db + Row 数据访问 | `Db` 门面 + `Row` + `#[derive(Model)]` | 异步原生 + 自研驱动（PostgreSQL / MySQL / SQLite） |
| SQL 模板（Enjoy SQL） | `Expr` + Builder + `sql!` + `ModelQuery` 终态动词 | 类型安全拼装，占位符随方言渲染 |
| 隐式事务提交 | `Drop` + `?` | 编译器强制执行，不可能「忘记回滚」 |
| Plugin 生命周期 | `Plugin` trait + 拓扑排序 | 异步 start/stop，编译期自动发现 |
| 类扫描 → 编译期注册 | `inventory` 编译期注册 | 零启动时间，毫秒级 |

**在数据访问层**，Alun 同样明确对标 Aifei 的调用风格：

- `Model.of(id).set(...).update()` —— 不先 `SELECT` 就能更新（Aifei 同款取舍）
- `deleteById` / `delete_in_ids` 的命名区分（Aifei 亦用 `deleteById` 区分实例侧 `delete`）
- `insertOrUpdate` 语义 → `PersistExt`（Alun 改为以存在性判定，更安全）
- `#where(...)` 动态条件 → `sql!` 宏（Alun 为**编译期校验**而非运行时反射）

详见 [`alun-db/README.md`](../../alun-db/README.md) 与 [`reference/crates/db.md`](../reference/crates/db.md)。

## 性能与体量对比

| 维度 | Aifei (Java) | Alun (Rust) | 变化 |
|------|-------------|------------|---------|
| HTTP 服务器 | Undertow (~4MB) | axum + hyper (~1.5MB) | 内存 −60% |
| 启动时间 | 1–3s（JVM 预热） | < 50ms | **20–60×** |
| 路由扫描 | classpath 遍历 | `inventory` 编译期注册 | 零运行时开销 |
| AOP | CGLib 字节码代理 | tower Layer | 类型安全 + 无运行时开销 |
| JSON | 反射 | serde 编译期 | **2–3×** |
| 事务安全 | 代码约定 | `Drop` + `?` | 编译器保证 |
| 并发模型 | 线程池阻塞 | tokio 异步 | DB 密集场景 **3–10×** |
| 部署体积 | JRE 200MB+ | 静态链接 ~10–20MB | **10–20×** |
| 运行时依赖 | JDK 17+ | 无 | 绿色部署 |

## 延伸阅读

- [architecture/overview.md](overview.md) —— Alun 完整分层架构与设计理念
- [guides/fullstack.md](../guides/fullstack.md) §4 —— Aifei → Alun 的项目架构映射
- [CHANGELOG.md](../../CHANGELOG.md) —— 相对早期版本的破坏性变更

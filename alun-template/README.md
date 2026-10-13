# alun-template

Alun 模板引擎抽象：基于 minijinja（Jinja2 语法）的服务端渲染。

> 门面侧 `features = ["template"]` 后，模板渲染走全局访问器
> `render_template(name, ctx)` / `try_template()` / `set_template(engine)`（由 `alun-web` 提供，`alun` 再导出）。

## 主要 API

`TemplateEngine` —— 对 `minijinja::Environment` 的薄封装（模板在启动时加载一次）：

- `TemplateEngine::new()` —— 空引擎（仅 `render_str` 可用）
- `TemplateEngine::from_dir(dir)` —— 以 `dir` 为根注册路径加载器
- `render<C: Serialize>(name, &ctx) -> alun_core::Result<String>` —— 按名渲染
- `render_str<C: Serialize>(source, &ctx) -> alun_core::Result<String>` —— 渲染内联模板
- 实现了 `Default`（等价于 `new()`）

`TemplateEngine` 是 `Clone`，设计上就只装一份当全局资源。

## 示例

```rust,ignore
use alun_template::TemplateEngine;

let engine = TemplateEngine::from_dir("templates")?;
let html = engine.render("index.html", &serde_json::json!({ "title": "Hello" }))?;

let inline = engine.render_str("Hello {{ name }}!", &serde_json::json!({ "name": "alun" }))?;
assert_eq!(inline, "Hello alun!");
```

装配进应用：

```rust,ignore
use alun::prelude::*;

let engine = TemplateEngine::from_dir("templates")?;
set_template(engine).expect("template engine already installed");

let html = render_template("index.html", &json!({ "title": "Hello" }))?;
```

> 模板里的 `.html` 后缀别漏：`render_template("index", ..)` 会找不到 `index.html`。

## Feature

本 crate 的 `Cargo.toml` 里没有 `[features]` 表；`askama` 是**可选依赖**
（隐式 feature 因此名为 `askama`）。当前实现的渲染引擎只有 minijinja ——
源码里并未引用 `askama`，开启它对 API 没有影响。

## 文档

- 主框架说明：[项目根 `README.md`](../README.md)
- 速查参考：`skill-alun/references/plugins.md`（模板一节）

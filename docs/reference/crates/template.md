---
id: reference.crates.template
title: `alun-template` — 模板引擎
summary: `alun-template` — 模板引擎——关键结构体、Trait、函数与 feature 说明；按 crate 精确查阅，不必通读全量参考。
tags: [template, reference, minijinja]
status: active
owner: alun-core-team
source_of_truth: [alun-template/src]
depends_on: [reference.crates]
last_reviewed: 2026-10-06
---

# `alun-template` — 模板引擎

> 本文件是 [`crates.md`](../crates.md) 的 `template` 模块分册；全景与索引见 [Crate 详细参考](../crates.md)。


**路径**: `alun-template/src/lib.rs`

封装 `minijinja`（Jinja2 语法），启动时一次性加载模板目录。

## `TemplateEngine`

| 方法                        | 说明                      |
| ------------------------- | ----------------------- |
| `new()`                   | 创建空引擎（仅支持 `render_str`） |
| `from_dir(dir)`           | 从目录加载模板                 |
| `render(name, ctx)`       | 渲染指定模板                  |
| `render_str(source, ctx)` | 从字符串渲染                  |

## 使用示例

```rust,ignore
use alun_template::TemplateEngine;

// ── 从目录加载 ──
let engine = TemplateEngine::from_dir("templates")?;

// ── 渲染模板文件 ──
use serde_json::json;
let html = engine.render("index.html", &json!({
    "title": "Alun",
    "users": [{"name": "张三"}, {"name": "李四"}],
}))?;

// ── 从字符串渲染 ──
let result = engine.render_str("Hello, {{ name }}!", &json!({"name": "World"}))?;

// ── 在 Handler 中使用（需 template feature） ──
async fn home() -> Result<Res<String>, ApiError> {
    let html = render_template("index.html", &json!({"title": "Home"}))
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(Res::ok(html))
}
```

**模板示例** (`templates/index.html`):

```html
<!DOCTYPE html>
<html>
<head><title>{{ title }}</title></head>
<body>
    <h1>{{ title }}</h1>
    <ul>
        {% for user in users %}
        <li>{{ user.name }}</li>
        {% endfor %}
    </ul>
</body>
</html>
```

***

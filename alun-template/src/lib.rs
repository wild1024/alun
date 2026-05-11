//! Template engine abstraction — 运行时模板用 minijinja

use std::sync::Arc;
use alun_core::Result;

/// 模板引擎（封装 minijinja，启动时一次性加载模板目录）
#[derive(Clone)]
pub struct TemplateEngine {
    /// minijinja 环境（持有模板编译后的模板）
    env: Arc<minijinja::Environment<'static>>,
}

impl TemplateEngine {
    /// 创建空引擎（不加载任何模板，仅支持 `render_str`）
    pub fn new() -> Self {
        Self { env: Arc::new(minijinja::Environment::new()) }
    }

    /// 加载模板目录，模板通过文件名引用（如 "index.html", "dashboard.html"）
    pub fn from_dir(dir: &str) -> Result<Self> {
        let mut env = minijinja::Environment::new();
        env.set_loader(minijinja::path_loader(dir));
        Ok(Self { env: Arc::new(env) })
    }

    /// 渲染指定模板
    ///
    /// # 参数
    /// - `name`: 模板文件名（如 "index.html"），需先用 `from_dir` 加载目录
    /// - `ctx`: 模板上下文（任何实现 Serialize 的类型）
    pub fn render<C: serde::Serialize>(&self, name: &str, ctx: &C) -> Result<String> {
        let tmpl = self.env
            .get_template(name)
            .map_err(|e| alun_core::Error::Template(format!("模板加载失败 {}: {}", name, e)))?;
        let ctx_val = minijinja::value::Value::from_serialize(ctx);
        tmpl.render(&ctx_val)
            .map_err(|e| alun_core::Error::Template(format!("模板渲染失败: {}", e)))
    }

    /// 从字符串渲染（用于动态模板内容）
    pub fn render_str<C: serde::Serialize>(&self, source: &str, ctx: &C) -> Result<String> {
        let tmpl = self.env
            .template_from_str(source)
            .map_err(|e| alun_core::Error::Template(format!("模板编译失败: {}", e)))?;
        let ctx_val = minijinja::value::Value::from_serialize(ctx);
        tmpl.render(&ctx_val)
            .map_err(|e| alun_core::Error::Template(format!("模板渲染失败: {}", e)))
    }
}

impl Default for TemplateEngine {
    fn default() -> Self { Self::new() }
}

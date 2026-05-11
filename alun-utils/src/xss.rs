//! HTML/XSS 净化工具（按需，仅当后端返回用户富文本时需要）
//!
//! # 使用示例
//!
//! ```ignore
//! use alun_utils::xss;
//!
//! let safe = xss::sanitize_html("<script>alert(1)</script><p>Hello</p>");
//! assert_eq!(safe, "<p>Hello</p>");
//! ```

use ammonia::Builder;

/// 使用默认规则净化 HTML
///
/// 移除 `<script>`、`<style>`、事件属性（`onclick` 等）、
/// javascript: URL 等危险内容，只保留安全标签和属性。
///
/// # 默认允许的标签
///
/// - 文本格式：`b`, `i`, `em`, `strong`, `u`, `s`, `code`, `pre`, `blockquote`
/// - 段落：`p`, `br`, `hr`
/// - 标题：`h1` ~ `h6`
/// - 链接：`a`（仅 `href`, `title`, `target` 属性）
/// - 列表：`ul`, `ol`, `li`
/// - 表格：`table`, `thead`, `tbody`, `tr`, `th`, `td`
/// - 图片：`img`（仅 `src`, `alt`, `width`, `height`）
/// - 容器：`div`, `span`
pub fn sanitize_html(html: &str) -> String {
    Builder::default()
        .clean(html)
        .to_string()
}

/// 使用严格规则净化 HTML（仅保留基本文本格式标签）
pub fn sanitize_html_strict(html: &str) -> String {
    Builder::default()
        .strip_all_tags()
        .clean(html)
        .to_string()
}

/// 检查 HTML 中是否包含潜在 XSS 载荷
///
/// 返回 `true` 表示可能包含恶意内容（需要净化处理）。
pub fn has_potential_xss(html: &str) -> bool {
    let cleaned = Builder::default().clean(html).to_string();
    cleaned.len() < html.len() || cleaned != html
}
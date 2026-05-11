//! alun-template 功能测试
//!
//! 覆盖：模板加载、渲染、render_str

#[cfg(test)]
mod tests {
    use alun_template::TemplateEngine;
    use serde_json::json;

    // ──── TemplateEngine::new ─────────────────────────

    #[test]
    fn test_template_engine_new() {
        let engine = TemplateEngine::new();
        assert!(engine.render_str("hello", &json!({})).is_ok());
    }

    // ──── render_str ─────────────────────────────────

    #[test]
    fn test_render_str_simple() {
        let engine = TemplateEngine::new();
        let result = engine.render_str("Hello, {{ name }}!", &json!({"name": "World"}));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Hello, World!");
    }

    #[test]
    fn test_render_str_with_loops() {
        let engine = TemplateEngine::new();
        let source = "Items: {% for item in items %}{{ item }}{% if not loop.last %}, {% endif %}{% endfor %}";
        let result = engine.render_str(source, &json!({"items": ["a", "b", "c"]}));
        assert_eq!(result.unwrap(), "Items: a, b, c");
    }

    #[test]
    fn test_render_str_with_conditionals() {
        let engine = TemplateEngine::new();
        let source = "{% if is_admin %}Admin{% else %}User{% endif %}";
        let result = engine.render_str(source, &json!({"is_admin": true}));
        assert_eq!(result.unwrap(), "Admin");

        let result = engine.render_str(source, &json!({"is_admin": false}));
        assert_eq!(result.unwrap(), "User");
    }

    #[test]
    fn test_render_str_escaping() {
        let engine = TemplateEngine::new();
        let source = "{{ content|safe }}";
        let result = engine.render_str(source, &json!({"content": "<b>bold</b>"}));
        let html = result.unwrap();
        assert!(html.contains("<b>bold</b>"));
    }

    #[test]
    fn test_render_str_missing_var() {
        let engine = TemplateEngine::new();
        let result = engine.render_str("{{ missing_var|default(\"N/A\") }}", &json!({}));
        assert_eq!(result.unwrap(), "N/A");
    }

    #[test]
    fn test_render_str_empty() {
        let engine = TemplateEngine::new();
        let result = engine.render_str("", &json!({}));
        assert_eq!(result.unwrap(), "");
    }

    #[test]
    fn test_render_str_nested_objects() {
        let engine = TemplateEngine::new();
        let source = "{{ user.name }} is {{ user.age }} years old";
        let ctx = json!({"user": {"name": "Alice", "age": 30}});
        let result = engine.render_str(source, &ctx);
        assert_eq!(result.unwrap(), "Alice is 30 years old");
    }

    #[test]
    fn test_render_str_multiline() {
        let engine = TemplateEngine::new();
        let source = "Line1\n{{ var }}\nLine3";
        let result = engine.render_str(source, &json!({"var": "Line2"}));
        assert_eq!(result.unwrap(), "Line1\nLine2\nLine3");
    }

    // ──── render ─────────────────────────────────────

    #[test]
    fn test_render_no_template_dir() {
        let engine = TemplateEngine::new();
        let result = engine.render("nonexistent.html", &json!({}));
        assert!(result.is_err());
    }

    // ──── Default impl ───────────────────────────────

    #[test]
    fn test_template_engine_default() {
        let engine = TemplateEngine::default();
        let result = engine.render_str("{{ x }}", &json!({"x": 42}));
        assert_eq!(result.unwrap(), "42");
    }
}
//! alun-core 功能测试
//!
//! 覆盖：Error、Res、ApiError、PageQuery、Plugin

#[cfg(test)]
mod tests {
    use alun_core::api::{Res, ApiError, PageQuery, PageData, codes};
    use alun_core::{Error, Result};
    use alun_core::plugin::{Plugin, PluginManager};
    use serde::Serialize;

    // ──── Error 类型测试 ──────────────────────────────

    #[test]
    fn test_error_display() {
        let e = Error::Config("字段缺失".into());
        assert!(e.to_string().contains("配置错误"));
        assert!(e.to_string().contains("字段缺失"));
    }

    #[test]
    fn test_error_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "文件不存在");
        let e = Error::Io(io_err);
        assert!(e.to_string().contains("IO 错误"));
        assert!(e.to_string().contains("文件不存在"));
    }

    #[test]
    fn test_error_server() {
        let e = Error::Server("端口被占用".into());
        assert!(e.to_string().contains("服务器错误"));
    }

    #[test]
    fn test_error_template() {
        let e = Error::Template("渲染失败".into());
        assert!(e.to_string().contains("模板错误"));
    }

    #[test]
    fn test_error_msg() {
        let e = Error::Msg("通用错误".into());
        assert_eq!(e.to_string(), "通用错误");
    }

    #[test]
    fn test_error_plugin() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "plugin panic");
        let e = Error::Plugin {
            name: "test_plugin".into(),
            source: Box::new(io_err),
        };
        assert!(e.to_string().contains("test_plugin"));
        assert!(e.to_string().contains("plugin panic"));
    }

    #[test]
    fn test_error_source() {
        use std::error::Error as StdError;
        let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "perm");
        let e = Error::Io(io_err);
        assert!(StdError::source(&e).is_some());
    }

    #[test]
    fn test_error_from_string() {
        let e: Error = String::from("测试错误").into();
        assert_eq!(e.to_string(), "测试错误");
    }

    #[test]
    fn test_error_from_str() {
        let e: Error = "静态错误".into();
        assert_eq!(e.to_string(), "静态错误");
    }

    #[test]
    fn test_error_from_io() {
        let io: std::io::Error = std::io::Error::from(std::io::ErrorKind::TimedOut);
        let e: Error = io.into();
        assert!(matches!(e, Error::Io(_)));
    }

    #[test]
    fn test_result_alias() {
        let r: Result<i32> = Ok(42);
        assert_eq!(r.unwrap(), 42);

        let e: Result<i32> = Err(Error::Msg("失败".into()));
        assert!(e.is_err());
    }

    // ──── Res 响应体测试 ──────────────────────────────

    #[test]
    fn test_res_ok_with_data() {
        let r = Res::ok("hello");
        assert_eq!(r.code, codes::OK);
        assert_eq!(r.msg, "ok");
        assert_eq!(r.data, Some("hello"));
    }

    #[test]
    fn test_res_ok_empty() {
        let r = Res::<()>::ok_empty();
        assert_eq!(r.code, codes::OK);
        assert_eq!(r.msg, "ok");
        assert!(r.data.is_none());
    }

    #[test]
    fn test_res_ok_msg() {
        let r = Res::<()>::ok_msg("操作成功");
        assert_eq!(r.code, codes::OK);
        assert_eq!(r.msg, "操作成功");
        assert!(r.data.is_none());
    }

    #[test]
    fn test_res_ok_with_msg() {
        let r = Res::ok_with_msg(123, "创建成功");
        assert_eq!(r.code, codes::OK);
        assert_eq!(r.msg, "创建成功");
        assert_eq!(r.data, Some(123));
    }

    #[test]
    fn test_res_fail() {
        let r = Res::<()>::fail(codes::BAD_REQUEST, "参数错误");
        assert_eq!(r.code, codes::BAD_REQUEST);
        assert_eq!(r.msg, "参数错误");
        assert!(r.data.is_none());
    }

    #[test]
    fn test_res_page() {
        #[derive(Debug, Clone, Serialize, PartialEq)]
        struct Item { id: i64, name: String }

        let items = vec![
            Item { id: 1, name: "A".into() },
            Item { id: 2, name: "B".into() },
        ];
        let r = Res::page(items, 100, 1, 20);
        assert_eq!(r.code, codes::OK);
        assert!(r.data.is_some());
        let page = r.data.unwrap();
        assert_eq!(page.total, 100);
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, 20);
        assert_eq!(page.list.len(), 2);
    }

    #[test]
    fn test_res_serialization() {
        let r = Res::ok("test");
        let json = serde_json::to_string(&r).unwrap();
        assert!(json.contains("\"code\":0"));
        assert!(json.contains("\"msg\":\"ok\""));
        assert!(json.contains("\"data\":\"test\""));
    }

    #[test]
    fn test_res_serialization_no_data() {
        let r = Res::<()>::ok_empty();
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("\"data\""));
    }

    // ──── ApiError 测试 ───────────────────────────────

    #[test]
    fn test_api_error_new() {
        let err = ApiError::new(418, 999, "I'm a teapot");
        assert_eq!(err.status, 418);
        assert_eq!(err.code, 999);
        assert_eq!(err.msg, "I'm a teapot");
    }

    #[test]
    fn test_api_error_bad_request() {
        let err = ApiError::bad_request("字段缺失");
        assert_eq!(err.status, 400);
        assert_eq!(err.code, codes::BAD_REQUEST);
    }

    #[test]
    fn test_api_error_unauthorized() {
        let err = ApiError::unauthorized("未登录");
        assert_eq!(err.status, 401);
        assert_eq!(err.code, codes::UNAUTHORIZED);
    }

    #[test]
    fn test_api_error_forbidden() {
        let err = ApiError::forbidden("权限不足");
        assert_eq!(err.status, 403);
        assert_eq!(err.code, codes::FORBIDDEN);
    }

    #[test]
    fn test_api_error_not_found() {
        let err = ApiError::not_found("用户不存在");
        assert_eq!(err.status, 404);
        assert_eq!(err.code, codes::NOT_FOUND);
    }

    #[test]
    fn test_api_error_method_not_allowed() {
        let err = ApiError::method_not_allowed("不支持的方法");
        assert_eq!(err.status, 405);
        assert_eq!(err.code, codes::METHOD_NOT_ALLOWED);
    }

    #[test]
    fn test_api_error_conflict() {
        let err = ApiError::conflict("用户名已存在");
        assert_eq!(err.status, 409);
        assert_eq!(err.code, codes::CONFLICT);
    }

    #[test]
    fn test_api_error_unprocessable_entity() {
        let err = ApiError::unprocessable_entity("字段校验失败");
        assert_eq!(err.status, 422);
        assert_eq!(err.code, codes::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn test_api_error_too_many_requests() {
        let err = ApiError::too_many_requests("请求过于频繁");
        assert_eq!(err.status, 429);
        assert_eq!(err.code, codes::TOO_MANY_REQUESTS);
    }

    #[test]
    fn test_api_error_internal() {
        let err = ApiError::internal("服务器异常");
        assert_eq!(err.status, 500);
        assert_eq!(err.code, codes::INTERNAL);
    }

    #[test]
    fn test_api_error_internal_masked() {
        let err = ApiError::internal_masked("系统繁忙", "detailed stack trace");
        assert_eq!(err.status, 500);
        assert_eq!(err.code, codes::INTERNAL);
        assert_eq!(err.msg, "系统繁忙");
        assert_eq!(err.internal_detail, Some("detailed stack trace".into()));
    }

    #[test]
    fn test_api_error_service_unavailable() {
        let err = ApiError::service_unavailable("维护中");
        assert_eq!(err.status, 503);
        assert_eq!(err.code, codes::SERVICE_UNAVAILABLE);
    }

    #[test]
    fn test_api_error_from_error() {
        let e = Error::Msg("底层异常".into());
        let api: ApiError = e.into();
        assert_eq!(api.status, 500);
        assert_eq!(api.code, codes::INTERNAL);
        assert!(api.internal_detail.is_some());
    }

    // ──── PageQuery 测试 ──────────────────────────────

    #[test]
    fn test_page_query_normal() {
        let pq = PageQuery::new(2, 30);
        assert_eq!(pq.page, 2);
        assert_eq!(pq.page_size, 30);
    }

    #[test]
    fn test_page_query_clamp_page() {
        let pq = PageQuery::new(0, 20);
        assert_eq!(pq.page, 1);
    }

    #[test]
    fn test_page_query_clamp_page_size_min() {
        let pq = PageQuery::new(1, 0);
        assert_eq!(pq.page_size, 10);
    }

    #[test]
    fn test_page_query_clamp_page_size_max() {
        let pq = PageQuery::new(1, 2000);
        assert_eq!(pq.page_size, 1000);
    }

    #[test]
    fn test_page_query_offset() {
        let pq = PageQuery::new(3, 20);
        assert_eq!(pq.offset(), 40);
    }

    #[test]
    fn test_page_query_offset_first_page() {
        let pq = PageQuery::new(1, 20);
        assert_eq!(pq.offset(), 0);
    }

    #[test]
    fn test_page_query_limit() {
        let pq = PageQuery::new(2, 50);
        assert_eq!(pq.limit(), 50);
    }

    // ──── codes 常量测试 ──────────────────────────────

    #[test]
    fn test_codes_values() {
        assert_eq!(codes::OK, 0);
        assert_eq!(codes::BAD_REQUEST, 400);
        assert_eq!(codes::UNAUTHORIZED, 401);
        assert_eq!(codes::FORBIDDEN, 403);
        assert_eq!(codes::NOT_FOUND, 404);
        assert_eq!(codes::METHOD_NOT_ALLOWED, 405);
        assert_eq!(codes::CONFLICT, 409);
        assert_eq!(codes::UNPROCESSABLE_ENTITY, 422);
        assert_eq!(codes::TOO_MANY_REQUESTS, 429);
        assert_eq!(codes::INTERNAL, 500);
        assert_eq!(codes::SERVICE_UNAVAILABLE, 503);
    }

    // ──── Plugin 系统测试 ─────────────────────────────

    struct TestPlugin {
        name: &'static str,
    }

    #[async_trait::async_trait]
    impl Plugin for TestPlugin {
        fn name(&self) -> &str { self.name }

        async fn start(&self) -> Result<()> {
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            Ok(())
        }

        fn depends_on(&self) -> &[&str] {
            &[]
        }
    }

    #[test]
    fn test_plugin_manager_new() {
        let pm = PluginManager::new();
        let result = pm.check_duplicate_names();
        assert!(result.is_ok());
    }

    #[test]
    fn test_plugin_manager_add() {
        let plugin = TestPlugin { name: "test1" };
        let pm = PluginManager::new().add(plugin);
        let result = pm.check_duplicate_names();
        assert!(result.is_ok());
    }

    #[test]
    fn test_plugin_manager_duplicate_names() {
        let pm = PluginManager::new()
            .add(TestPlugin { name: "dup" })
            .add(TestPlugin { name: "dup" });
        let result = pm.check_duplicate_names();
        assert!(result.is_err());
    }

    // ──── PageData 测试 ───────────────────────────────

    #[test]
    fn test_page_data_serialization() {
        let pd = PageData {
            list: vec!["a", "b", "c"],
            total: 30,
            page: 2,
            page_size: 15,
        };
        let json = serde_json::to_string(&pd).unwrap();
        assert!(json.contains("\"total\":30"));
        assert!(json.contains("\"page\":2"));
        assert!(json.contains("\"page_size\":15"));
        assert!(json.contains("\"a\""));
    }

    #[test]
    fn test_res_fail_with_various_codes() {
        let codes_to_test = vec![
            (codes::BAD_REQUEST, 400),
            (codes::UNAUTHORIZED, 401),
            (codes::FORBIDDEN, 403),
            (codes::NOT_FOUND, 404),
            (codes::INTERNAL, 500),
        ];

        for (code, _) in codes_to_test {
            let r = Res::<()>::fail(code, "测试失败");
            assert_eq!(r.code, code);
            assert_eq!(r.msg, "测试失败");
            assert!(r.data.is_none());
        }
    }

    #[test]
    fn test_res_json_serialization() {
        use serde_json::json;

        let value = json!({"key": "value", "num": 42});
        let r = Res::ok(value);
        let json_str = serde_json::to_string(&r).unwrap();
        assert!(json_str.contains("\"code\":0"));
        assert!(json_str.contains("\"key\":\"value\""));
        assert!(json_str.contains("\"num\":42"));
    }
}
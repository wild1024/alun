//! Alun 框架完整测试套件
//!
//! # 测试分类
//!
//! ## 功能测试（functional）
//! - `core_tests` — alun-core：Error、Res、ApiError、PageQuery、Plugin
//! - `utils_tests` — alun-utils：Crypto、Sid、Mask、Valid、Date、Str、Export、Web
//! - `config_tests` — alun-config：AppConfig、多环境、ConfigManager
//! - `cache_tests` — alun-cache：LocalCache 完整生命周期
//! - `template_tests` — alun-template：模板渲染
//! - `fs_tests` — alun-fs：文件读写
//! - `web_tests` — alun-web：Router、JWT、App 构建器
//! - `middleware_tests` — 中间件单元测试：Auth、Permission、Role、RateLimit、SecurityHeaders、Nonce、Idempotency、RequestId
//! - `plugin_tests` — 插件系统测试：生命周期、依赖拓扑排序、循环检测
//!
//! ## 真实场景测试（scenarios）
//! - `real_world_tests` — 用户注册/登录、CRUD、JWT 认证鉴权、分页、缓存
//! - `middleware_scenarios` — 中间件集成场景：认证链、权限链、限流链
//! - `plugin_scenarios` — 插件集成场景：启动/停止顺序验证
//!
//! ## 安全/漏洞测试（security）
//! - `vulnerability_tests` — XSS注入、SQL注入、JWT攻击、路径遍历、CSRF、限流绕过、IDOR、暴力破解防护
//!
//! ## 压力/性能测试（stress）
//! - `stress_tests` — 并发请求、大载荷、持续负载、限流高压、内存使用
//!
//! 运行方式：
//! ```bash
//! cargo test -p alun-test-suite
//! cargo test -p alun-test-suite -- --nocapture
//! ```

pub mod functional;
pub mod scenarios;
pub mod security;
pub mod stress;

fn main() {
    println!("Alun 测试套件 —— 使用 cargo test 运行");
    println!("  cargo test -p alun-test-suite");
    println!("  cargo test -p alun-test-suite -- --nocapture");
}
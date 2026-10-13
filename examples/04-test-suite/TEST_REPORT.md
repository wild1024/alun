# Alun 框架测试报告

## 概述

本文档为 Alun Rust Web 框架的完整测试报告，覆盖功能测试、中间件测试、插件测试、安全漏洞测试和压力测试五大类别。

- **测试项目**：`examples/04-test-suite`
- **测试框架**：`#[tokio::test]` / `#[test]` + `tower::ServiceExt::oneshot`（直接调用 axum `Router`，无需监听端口）
- **测试总数**：347 个
- **通过率**：100%（347 通过 / 0 失败 / 0 忽略）
- **执行时间**：约 2 秒（此前本机实测值，仅供参考）

---

## 一、中间件单元测试（30 项）

测试文件：`src/functional/middleware_tests.rs`

### AuthLayer（认证中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_auth_layer_valid_token` | 有效 JWT Token 验证通过 | PASS |
| `test_auth_layer_invalid_token_returns_401` | 无效 Token 返回 401 | PASS |
| `test_auth_layer_no_token_returns_401` | 缺少 Token 返回 401 | PASS |
| `test_auth_layer_expired_token_returns_401` | 过期 Token 处理（已知 leeway=60s 限制） | PASS |
| `test_auth_layer_wrong_secret_returns_401` | 错误密钥签名返回 401 | PASS |
| `test_auth_layer_ignore_path` | 配置的忽略路径跳过认证 | PASS |
| `test_auth_layer_injects_claims` | Claims 正确注入到 Extension | PASS |

### RequirePermissionLayer（权限中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_permission_layer_has_permission` | 拥有所需权限时通过 | PASS |
| `test_permission_layer_no_permission_returns_403` | 缺少权限时返回 403 | PASS |
| `test_permission_layer_super_admin` | 超级管理员绕过权限检查 | PASS |
| `test_permission_layer_no_auth_returns_401` | 未认证时返回 401 | PASS |

### RequireRoleLayer（角色中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_role_layer_has_role` | 拥有所需角色时通过 | PASS |
| `test_role_layer_wrong_role_returns_403` | 角色不匹配时返回 403 | PASS |

### RateLimitLayer（限流中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_rate_limit_allows_within_budget` | 预算内的请求正常通过 | PASS |
| `test_rate_limit_blocks_exceeding_requests` | 超出预算的请求返回 429 | PASS |
| `test_rate_limit_different_ips_independent` | 不同 IP 的限流独立计算 | PASS |

### SecurityHeadersLayer（安全头中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_security_headers_nosniff` | `X-Content-Type-Options: nosniff` | PASS |
| `test_security_headers_frame_deny` | `X-Frame-Options: DENY` | PASS |
| `test_security_headers_hsts` | `Strict-Transport-Security` 头 | PASS |
| `test_security_headers_csp` | `Content-Security-Policy` 头 | PASS |

### NonceLayer（防重放中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_nonce_first_request_succeeds` | 首次请求（含有效 Nonce）通过 | PASS |
| `test_nonce_replay_returns_409` | 重复 Nonce 返回 409 | PASS |
| `test_nonce_without_header_succeeds` | 缺少 Nonce 头通过（兼容模式） | PASS |

### IdempotencyLayer（幂等中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_idempotency_first_request_succeeds` | 首次幂等请求通过 | PASS |
| `test_idempotency_replay_returns_cached` | 重复请求返回缓存响应 | PASS |
| `test_idempotency_different_keys` | 不同幂等键的请求独立处理 | PASS |

### RequestIdLayer（请求 ID 中间件）

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_request_id_layer_generates_id` | 请求未带 ID 时中间件生成并注入请求头（断言非空） | PASS |
| `test_request_id_preserves_existing` | 保留客户端传入的请求 ID | PASS |

### TokenClaims 安全边界

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_token_claims_super_admin_has_all` | 超管拥有所有权限和角色 | PASS |
| `test_token_claims_regular_user_restricted` | 普通用户权限受限于声明的值 | PASS |

---

## 二、插件系统测试（10 项）

测试文件：`src/functional/plugin_tests.rs`

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_plugin_start_stop_order` | 启动/停止顺序与注册顺序一致 | PASS |
| `test_plugin_dependency_order` | 按依赖拓扑排序：db → cache → web | PASS |
| `test_plugin_cycle_detection` | 循环依赖（Kahn 算法）检测 | PASS |
| `test_plugin_start_failure_aborts` | 插件启动失败时后续插件不会启动 | PASS |
| `test_plugin_stop_continues_after_failure` | 插件停止失败时继续停止后续插件 | PASS |
| `test_plugin_duplicate_name` | 同名插件注册检测 | PASS |
| `test_empty_plugin_manager` | 空管理器行为 | PASS |
| `test_plugin_missing_dependency` | 依赖不存在的插件时检测 | PASS |
| `test_plugin_multiple_dependency_chains` | 多条依赖链并行排序 | PASS |
| `test_plugin_self_dependency` | 自我依赖检测 | PASS |

---

## 三、中间件链场景测试（8 项）

测试文件：`src/scenarios/middleware_scenarios.rs`

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_full_auth_permission_chain` | AuthLayer + RequirePermissionLayer 双重认证 | PASS |
| `test_auth_permission_chain_no_permission` | 认证通过但权限不匹配→403 | PASS |
| `test_auth_role_permission_chain` | AuthLayer + RequireRoleLayer + RequirePermissionLayer 三重认证 | PASS |
| `test_auth_role_chain_wrong_role` | 角色不匹配→403 | PASS |
| `test_security_chain_request_id_and_headers` | RequestIdLayer + SecurityHeadersLayer 安全头组合 | PASS |
| `test_rate_limit_auth_chain` | RateLimitLayer + AuthLayer 限流+认证组合 | PASS |
| `test_nonce_auth_chain_replay_prevention` | NonceLayer + AuthLayer 防重放+认证组合 | PASS |
| `test_idempotency_auth_chain` | IdempotencyLayer + AuthLayer 幂等+认证组合（3 次重复请求均成功） | PASS |

---

## 四、插件场景测试（3 项）

测试文件：`src/scenarios/plugin_scenarios.rs`

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_typical_app_plugin_stack` | 典型应用栈：database→cache→kafka→task_worker→file_storage→web_server，验证依赖关系 | PASS |
| `test_deep_dependency_chain` | 深层依赖链：l0→l1→l2→l3→l4 | PASS |
| `test_multi_branch_dependency_graph` | 多分支依赖：root_a→branch_a1/a2, root_b→branch_b1, merge 依赖 branch_a2+branch_b1 | PASS |

---

## 五、安全漏洞测试（19 项）

测试文件：`src/security/vulnerability_tests.rs`

### XSS 跨站脚本攻击

| 测试用例 | 攻击方式 | 结果 |
|---|---|---|
| `test_xss_script_tag_in_param_reflected` | URL 参数注入 `<script>alert('XSS')</script>`，验证服务不崩溃且返回合法 JSON | PASS |
| `test_xss_json_body_script_tag` | JSON 体注入 `<img src=x onerror=alert(1)>`，验证服务不崩溃 | PASS |
| `test_xss_svg_onload_injection` | JSON 体注入 `<svg onload=alert(1)>`，验证服务不崩溃 | PASS |

### SQL 注入

| 测试用例 | 攻击方式 | 结果 |
|---|---|---|
| `test_sql_injection_param_binding` | `' OR '1'='1` —— SQLite 参数化查询天然免疫 | PASS |
| `test_sql_injection_union_select` | `' UNION SELECT 'hacked'` —— 参数绑定阻止注入 | PASS |
| `test_sql_injection_drop_table` | `'; DROP TABLE users--` —— 被当作普通字符串 | PASS |

### JWT 攻击

| 测试用例 | 攻击方式 | 结果 |
|---|---|---|
| `test_jwt_attack_none_algorithm` | `alg: "none"` —— Axum 拒绝无签名 Token | PASS |
| `test_jwt_attack_forged_subject` | 伪造 `sub` 字段——Authorisation 头无效 | PASS |
| `test_jwt_attack_rs256_to_hs256` | RS256→HS256 算法混淆——无效签名拒绝 | PASS |

### 路径遍历

| 测试用例 | 攻击方式 | 结果 |
|---|---|---|
| `test_path_traversal_dot_dot` | `../../etc/passwd` —— 路径验证拒绝 | PASS |
| `test_path_traversal_url_encoded` | `%2e%2e%2f%2e%2e%2fetc/passwd` —— URL 解码后拒绝 | PASS |

### 其他攻击

| 测试用例 | 攻击方式 | 结果 |
|---|---|---|
| `test_rate_limit_bypass_with_varied_ips` | 多 IP 绕过限流——不同 IP 独立计数（正常行为） | PASS |
| `test_rate_limit_shared_router_state` | 共享 Router 状态下的限流共享 | PASS |
| `test_idor_cross_user_article_access` | 横向越权访问其他用户文章——403 | PASS |
| `test_csrf_origin_header_mismatch` | Origin 头不匹配——示例校验逻辑返回 403 | PASS |
| `test_csrf_referer_cross_origin` | 外部 Referer 头——框架当前不校验 Referer，请求正常通过（已知限制） | PASS |
| `test_brute_force_login_attempts` | 10 次连续登录试错——不崩溃 | PASS |
| `test_null_byte_injection` | Null 字节注入文件路径——拒绝 | PASS |
| `test_extremely_long_input` | 100KB 查询参数——不崩溃（断言响应为 2xx 或 4xx） | PASS |

---

## 六、压力测试（9 项）

测试文件：`src/stress/stress_tests.rs`

| 测试用例 | 场景 | 结果 |
|---|---|---|
| `test_concurrent_requests_stress` | 100 并发请求（多线程运行时），验证所有响应为 200 | PASS |
| `test_large_payload_handling` | 1000 条数据 × 200 字符的 JSON 数组处理 | PASS |
| `test_very_large_payload_rejected` | 5MB 请求体——被 axum 默认 body 上限拒绝（413），不产生 5xx | PASS |
| `test_sustained_load_stress` | 持续负载：10 轮 × 50 请求 = 500 次，全部通过 | PASS |
| `test_rate_limiter_under_pressure` | 限流器压力：60 并发，20 通过 + 40 被限 | PASS |
| `test_database_concurrent_reads` | 30 并发数据库读取 | PASS |
| `test_cache_concurrent_writes` | 50 任务 × 100 写入 = 5000 缓存条目 | PASS |
| `test_cache_concurrent_read_write` | 20 任务并发读写（混合负载） | PASS |
| `test_full_middleware_chain_stress` | 50 并发通过全中间件链（Idempotency+RateLimit+SecurityHeaders+RequestId） | PASS |

---

## 七、Phase 1 既有测试（268 项）

计数口径（按测试文件）：`core_tests` 51 + `utils_tests` 86 + `config_tests` 35 + `cache_tests` 28 + `template_tests` 11 + `fs_tests` 11 + `web_tests` 36 + `real_world_tests` 10 = 268。

### alun-core（51 项）

测试文件：`src/functional/core_tests.rs`

- `test_api_error_*`（13 项）：错误类型 → HTTP 状态码映射（400/401/403/404/405/409/422/429/500/503）
- `test_error_*`（10 项）：`Error` 展示、`source` 链、`From`（io/str/String）、插件/模板/服务端错误
- `test_res_*`（10 项）：`Res` 各类序列化模式（ok/ok_empty/ok_msg/ok_with_data/ok_with_msg/fail/page/无 data）
- `test_page_query_*` + `test_page_data_serialization`（13 项）：`PageQuery` 规整与越界保护、`offset()/limit()`、`PageData` 序列化
- `test_plugin_manager_*`（3 项）：注册、同名去重、空管理器
- `test_codes_values`、`test_result_alias`（2 项）

本轮新增的 5 个分页用例（均在 `src/functional/core_tests.rs`）：

| 测试用例 | 说明 | 结果 |
|---|---|---|
| `test_page_query_deserialize_clamps_zero` | 反序列化 `{"page":0,"page_size":0}` 规整为 `(1, 10)`，`offset()==0`、`limit()==10` | PASS |
| `test_page_query_deserialize_missing_fields` | 空对象 `{}` 反序列化得到默认 `(1, 10)`，缺参不再 400 | PASS |
| `test_page_query_deserialize_clamps_max` | `page_size=5000` 夹到 `MAX_PAGE_SIZE`（1000） | PASS |
| `test_page_query_offset_never_underflows` | 直接构造 `{page:0, page_size:0}` / `{u64::MAX, u64::MAX}` 也不 panic、offset 不下溢 | PASS |
| `test_page_query_calc_pages_matches_db` | `PageQuery::calc_pages` 与 `alun_db::Page::calc_pages` 口径一致（total=0/1/20/21/95） | PASS |

### alun-utils（86 项）

测试文件：`src/functional/utils_tests.rs`

- **加密**（16 项）：AES 加密/解密（有效/无效密钥/错误 nonce/空数据/Unicode）、SHA256、HMAC、密码哈希与校验、Base64URL、随机密钥/Token
- **标识符**（6 项）：UUID、UUIDv7、TSID、SID（长度档位/唯一性）
- **日期**（8 项）：相对日期（天/小时前/当前）、格式化、时间戳、当日首末时刻
- **验证**（9 项）：邮箱、手机号、IPv4、URL、密码强度、长度范围（含 Unicode）、字母数字/纯数字
- **掩码**（10 项）：手机号、邮箱、身份证、银行卡、姓名（含短值/单字边界）
- **数据**（8 项）：JSON/CSV 导入导出、XLSX 导出、JSON 值解析（合法/非法）
- **格式化**（6 项）：文件大小（Bytes/KB/MB/GB/0）、文件名清理
- **字符串**（6 项）：`StrExt` 随机生成、截断、清理、驼峰/蛇形转换、空白判定
- **Web**（7 项）：客户端 IP（`x-forwarded-for` 优先与回退）、私有 IP 判定、域名提取、路径提取、查询串构建
- **输入清理**（5 项）：登录/注册参数清理（邮箱、密码、字符串参数）

### alun-config（35 项）

测试文件：`src/functional/config_tests.rs`

- **默认值**（19 项）：`test_default_*`（18）+ `test_generate_default_config`——应用名、监听地址、日志、缓存、数据库、CORS、上传/下载、静态资源、模板、限流、路由前缀
- **配置结构与序列化**（13 项）：`AppConfig` TOML 序列化/反序列化、各配置段默认值（server/cache/log/cors/database/auth/中间件）、自定义 Router 前缀与 Server 覆盖、部分 TOML 回落默认、profile 探测
- **ConfigManager**（3 项）：静态读取、动态写入（含复杂值）、动态读写删除

### alun-cache（28 项）

测试文件：`src/functional/cache_tests.rs`

- `LocalCache` 基本 CRUD（9 项）：`set/get/del/overwrite`、字符串/数字/布尔/JSON 值、`len/is_empty`、缺失键
- TTL（6 项）：默认 TTL、`set_ex` 自定义 TTL 与过期、过期清理、`exists` 过期检测
- 存在性（2 项）：`exists` 真/假
- 计数（4 项）：`incr` 新建/已有/多次/负数
- Keys/删除模式（5 项）：`keys` 精确与通配、`delete_pattern` 单条/无匹配
- 统计与容量（2 项）：hits/misses/evictions/size、容量上限

### alun-template（11 项）

测试文件：`src/functional/template_tests.rs`

- `TemplateEngine` 创建与默认构造（2 项）
- `render_str` 渲染（7 项）：简单/条件/循环/嵌套对象/多行/转义/缺失变量
- 边界（2 项）：空模板字符串、模板目录不存在

### alun-fs（11 项）

测试文件：`src/functional/fs_tests.rs`

- `LocalFs` 创建与 root 路径（2 项）
- 写入/读取（3 项）：文本、二进制、指定文件名
- 删除（2 项）：存在与不存在
- 存在性检查（2 项）
- 读取不存在的文件（1 项）
- MIME 类型检测（图片）（1 项）

### alun-web（36 项）

测试文件：`src/functional/web_tests.rs`

- `App` 构建器（9 项）：默认配置、路由分组、`on_startup`、路由注册、方法注册、自定义配置、`ConfigManager`、角色/权限
- `Router`（13 项）：新建、默认/自定义方法、GET/POST/PUT/DELETE 注册、多路由/同路径多方法、合并、404、JSON 响应
- JWT（6 项）：访问令牌/刷新令牌创建与校验、无效令牌、错误密钥、Roles/Permissions 携带
- `TokenClaims`（8 项）：权限/角色判定、`is_super_admin`、超管全权限、`user_id`

### 真实场景测试（10 项）

测试文件：`src/scenarios/real_world_tests.rs`

- 完整注册→登录→访问流程
- 文章 CRUD 全流程（列表响应为分页结构，断言 `list/total/page/page_size`）
- JWT 完整生命周期
- 未授权访问拦截
- 无效 Token 处理
- 并发请求处理
- 分页查询（15 篇文章：第 1 页 10 条 / 第 2 页 5 条，`total=15`）
- 缓存集成
- 各类 Res 模式组合
- 错误处理全链路

---

## 八、关键安全发现

### 框架安全优势

1. **SQL 注入免疫**：SQLite 参数化查询本身阻止注入攻击，`' OR '1'='1` 等攻击负载被完整当作字符串处理
2. **JWT 算法防护**：无效 Token（伪造签名、错误算法）均返回 401，`alg: none` 被框架拒绝
3. **路径遍历防护**：`..` 和 `%2e%2e%2f` 编码攻击在 URL 解码后被正确拦截
4. **IDOR 防护**：Authentication 中间件验证身份后，业务层比对 `author_id`，跨用户访问返回 403
5. **CSRF 检测（Origin）**：测试应用在业务层校验 `Origin` 头，跨域来源返回 403；`Referer` 头当前未被框架校验（见「已知限制」）

### 已知限制

1. **JWT Leeway=60s**：`jsonwebtoken` 默认 leeway 为 60 秒，`expire_secs: 0` 的 Token 在 60 秒内仍被接受（RFC 允许）
2. **RequestIdLayer 响应头回带**：中间件把请求 ID 注入请求 Extensions，并**回写响应头 `x-request-id`**（客户端传入时原样回带；未传时生成 16 位十六进制 ID）——原「不设置响应头」的已知限制已随 `alun-web` 的 `D004` 修复消除；本套用例仍只覆盖请求侧注入的断言
3. **JSON 序列化不转义 HTML**：`serde_json` 不对 `<script>` 等标签进行 HTML 实体转义（符合 JSON 规范）
4. **不校验 Referer**：`test_csrf_referer_cross_origin` 记录当前行为——外部 Referer 的请求依旧通过，CSRF 防护需业务自行补充

### 压力测试结论

| 场景 | 并发数 | 通过率 | 表现 |
|---|---|---|---|
| 基础并发 | 100 | 100% | 全部 200 |
| 持续负载 | 500（分 10 轮） | 100% | 无降级 |
| 限流压力 | 60 并发 | 33% 通过 / 67% 限流 | 正确执行 |
| 全链压力 | 50 并发 × 全部中间件 | 100% | 无退化 |
| 缓存写入 | 50 并发 × 5000 条目 | 100% | 无丢失 |
| 超大数据 | 5MB 请求体 | 0%（413 拒绝） | 不产生 5xx |

---

## 九、测试统计汇总

| 类别 | 测试数 | 通过 | 失败 |
|---|---|---|---|
| 中间件单元测试 | 30 | 30 | 0 |
| 插件系统测试 | 10 | 10 | 0 |
| 中间件链场景测试 | 8 | 8 | 0 |
| 插件场景测试 | 3 | 3 | 0 |
| 安全漏洞测试 | 19 | 19 | 0 |
| 压力测试 | 9 | 9 | 0 |
| Phase 1 既有测试 | 268 | 268 | 0 |
| **总计** | **347** | **347** | **0** |

---

## 运行方式

在仓库根目录（工作区根，`examples/04-test-suite` 的 `package.name = "alun-test-suite"`）执行：

```bash
# 全量运行（推荐）
cargo test -p alun-test-suite --all-features

# 查看输出（含 println!/tracing）
cargo test -p alun-test-suite --all-features -- --nocapture

# 只跑某一分类
cargo test -p alun-test-suite --all-features functional::
cargo test -p alun-test-suite --all-features security::
cargo test -p alun-test-suite --all-features stress::
```

统计口径：本文档的用例数按 `examples/04-test-suite/src/**/*.rs` 中 `#[test]` / `#[tokio::test]`（含 `#[tokio::test(flavor = "multi_thread")]`）属性逐个计数，测试辅助函数（如 `setup_sqlite_db`、`register_and_login`、`test_jwt`）不计数；各分类数字与 `src/functional`、`src/scenarios`、`src/security`、`src/stress` 下的测试文件一一对应。

`--all-features` 对本包无额外特性（依赖特性已在 `Cargo.toml` 中直接开启：`alun` 的 `db/cache/template/fs/config`，`alun-utils` 的 `validator-integration`），加上它只是保证与工作区其他包的统一调用方式。
---
id: guides.startup.config-reference
title: 附录：配置全字段参考
summary: config.toml 全部可配置字段的默认值、取值与作用，按 server/log/database/redis/cache/middleware/tenant 分组。
tags: [getting-started, config, reference, cheatsheet]
status: active
owner: alun-core-team
source_of_truth: [code, examples, config/config.toml]
depends_on: [guides.getting-started]
last_reviewed: 2026-10-06
---

# 附录：配置全字段参考

> 本文件是 [`getting-started.md`](../getting-started.md) 的 `config-reference` 分册；总目录与全流程顺序见 [从零到跑起来](../getting-started.md)。

# 附录：配置全字段参考

> 新手可以先跳过这一节，需要精确控制某项行为时再回来查。

```toml
# ==================== 应用信息 ====================
# 注意：app_name / profile 是根级标量，没有 [app] 表
app_name = "Alun"
profile = "dev"              # 生效值由 profile=xxx 参数 / ALUN_PROFILE 决定

# ==================== 服务器 ====================
[server]
listen = "8023"              # 监听地址（端口号 或 ip:port）

# ==================== 日志 ====================
[log]
level = "info"               # trace | debug | info | warn | error
format = "text"              # text（彩色） | json（结构化）
dir = "logs"                 # 文件输出目录；注释掉则仅输出 stdout
file_prefix = "alun"         # 日滚文件名前缀

# ==================== 路由 ====================
[router]
prefix = ""                  # 全局路由前缀（如 "/api/v1"）

[router.not_found]
enabled = true               # 自定义 404（返回统一 JSON 错误响应）
message = "请求的资源不存在"

# ==================== 数据库 ====================
[database]
enabled = true
type = "postgres"            # postgres | mysql | sqlite
host = "localhost"
port = 5432                  # 可省略，缺省按类型取 5432 / 3306
name = "mydb"                # sqlite 时是数据库文件路径
user = "app_user"
password = ""
password_encrypted = false   # true 时密码为 AES-GCM 密文，用 ALUN_SECRET_KEY 解密
max_connections = 10
min_connections = 2
connect_timeout = 10         # 建连超时（秒）
ssl_mode = "prefer"          # postgres 专用：disable | prefer | require | verify-ca | verify-full
sql_logging = false
slow_query_ms = 0            # 慢查询阈值（毫秒），0 = 关闭

[database.migration]
enabled = false
path = "migrations"          # V{n}__Name.sql 或 V{n}__Name.{up,down}.sql
auto_migrate = false         # 启动时自动执行未应用的迁移

# ==================== Redis ====================
[redis]
enabled = false
url = "redis://127.0.0.1:6379"
max_connections = 10

# ==================== 缓存 ====================
[cache]
type = "local"               # local | redis
max_capacity = 10000         # 本地缓存最大条目数
default_ttl = 3600           # 默认过期秒数（0 = 永不过期）

# ==================== 中间件 ====================
[middleware]
request_id = false
request_log = false

[middleware.nonce]
enabled = false
ttl_secs = 300

[middleware.idempotency]
enabled = false
ttl_secs = 86400

[middleware.request_log_config]
exclude_paths = ["/health"]  # 精确匹配或通配符前缀（"/swagger*"）
log_duration = true

[middleware.auth]
enabled = false
jwt_secret = ""              # 也可用 ALUN_MIDDLEWARE_AUTH_JWT_SECRET / FACAI_JWT_SECRET 覆盖
ignore_paths = ["/api/login", "/api/register"]
access_token_expire_secs = 7200
refresh_token_expire_secs = 604800

[middleware.cors]
enabled = false
allow_origins = []
allow_methods = []
allow_headers = []
allow_credentials = true
max_age_secs = 86400

[middleware.compression]
enabled = false
level = 6                    # Gzip 压缩级别 0-9

[middleware.rate_limit]
enabled = false
requests_per_window = 100
window_secs = 60

[middleware.security_headers]
enabled = true               # 默认开启
nosniff = true
frame_options = true
hsts = true
hsts_max_age_secs = 31536000
hsts_include_subdomains = true
csp = true
csp_value = "default-src 'self'"
referrer_policy = true
referrer_policy_value = "strict-origin-when-cross-origin"
permissions_policy = false
permissions_policy_value = "camera=(), microphone=(), geolocation=()"

[middleware.permission]
enabled = false              # 仅控制配置文件里的 rules；#[alun::permission] 宏注解始终挂载
# rules = [
#   { path = "/api/admin/*", methods = ["GET"], permission = "admin:read" },
# ]

# ==================== 上传 / 下载 ====================
# max_size_mb 同时决定请求体硬顶：框架自动挂载 DefaultBodyLimit =
# max_size_mb MB + 32MiB（multipart 分帧 / 多字段余量），业务无需自行挂载。
[upload]
path = "uploads"
max_size_mb = 10

[download]
path = "downloads"

# ==================== 模板 ====================
[template]
path = "templates"

# ==================== 静态文件 ====================
[static_files]
enabled = false
path = "static"

# ==================== 插件 ====================
# enabled 是插件名单：自行用 alun_plugin::create_plugins_from_config(&cfg()) 构造，
# 或在 App 上逐个 .plugin(...) 注册
[plugins]
enabled = ["notification", "serial"]

[plugins.notification]
smtp_host = ""
smtp_port = 587
smtp_user = ""
smtp_pass = ""
from_email = ""              # 为空时使用 smtp_user
from_name = "系统通知"

[plugins.async_task]
workers = 4

[plugins.scheduler]
workers = 4

[plugins.serial]
backend = "memory"           # memory | redis | postgres
# rules = []                 # 静态单号规则（SerialRuleConfig 列表）

# ==================== 自定义配置 ====================
# custom 是 HashMap<String, serde_json::Value>，业务与插件自行读写
[custom]
# 例如 [custom.payment] / [custom.feature_flags]

# ==================== 文件存储（fs feature，未纳入 AppConfig） ====================
# 需自行读取：serde_json::from_value::<alun_fs::FsPluginConfig>(cfg().custom["fs"].clone())
[custom.fs]
default_backend_type = "local"   # local | minio | s3
local_root_dir = "uploads"
max_file_size_bytes = 52428800   # 默认 50MB
presign_url_ttl_secs = 3600

# ==================== 异步任务（task feature，从 custom 读取） ====================
# 代码侧：let task_cfg: TaskWorkerConfig = cfg().custom.get("task")
#         .and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default();
[custom.task]
brokers = "localhost:9092"
group_id = "alun-task-worker"
scan_interval_secs = 30
max_batch_size = 100
max_message_age_secs = 3600
auto_create_topics = false
topic_partitions = 1
topic_replication = 1
```

---

<!-- prev-next -->
| 上一章 | 下一章 |
| --- | --- |
| ← [⑥ 自定义配置](custom-config.md) | [附录：认证、鉴权与多租户安全模型](security-model.md) → |

<sub>本分册属于「从零到跑起来（8 步分册）」，共 8 篇。首篇 / 末篇可点「返回总目录」。</sub>

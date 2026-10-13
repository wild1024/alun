#!/usr/bin/env bash
# =============================================================================
#  Feature 纪律守卫 —— 防止默认依赖再次"变重"
# =============================================================================
#
#  背景：`alun` 曾长期使用 `default = ["full"]`，导致 `alun = "0.2"` 的
#  开箱体验是"必须装 CMake + C 工具链"（full → task → rdkafka → librdkafka，
#  而 librdkafka 的 configure 是 Unix shell 脚本，纯 Windows 下无法构建）。
#
#  现行约定（feature 分层，见 alun/Cargo.toml）：
#
#    L0 core      —— 无外部重量级依赖的 HTTP 能力（默认）
#    L1 能力       —— template / cache / plugin / fs（各自独立）
#    L2 db/驱动    —— 能力与驱动**分离**，驱动必须显式选
#    L2 fs 后端    —— fs-local / fs-minio 正交
#    L3 重型后端   —— kafka / task-kafka（需要 CMake + C 工具链）
#
#  预设（能力严格递增：mini ⊂ lite ⊂ full）：
#
#    mini —— core + sqlite + cache                                 零外部服务
#    lite —— core + postgres + cache + plugin + fs-local            单库生产（不含 template）
#    full —— lite + mysql + kafka + task-kafka + fs-minio + sse + metrics-prometheus
#                  + xlsx + template
#
#    错误出口**只有一个**：handler 签名恒为 `ResResult<T>`（`Result<Res<T>, ApiError>`）。
#    `ApiError` 是 `alun-web` 的本地类型，因此 `From<DbError> / From<alun_core::Error> /
#    From<io::Error> for ApiError` 全部合法，handler 里 `?` 即可直通（见 C6/C7）。
#    `alun-core` 与 `alun-db-core` 互不依赖是硬约束：核心层既不认识 HTTP
#    也不认识数据库，跨层转换一律落在同时认识两边的 `alun-web`。
#
#  本脚本断言以上约定未被破坏。任何一条不通过都应视为**发布阻断**。
#
#  用法：bash ci/check_feature_discipline.sh
# =============================================================================

set -uo pipefail

cd "$(dirname "$0")/.." || exit 1

# 重量级依赖的特征串：出现在「默认」依赖树中即为违规
#   - alun-db*           数据库及其驱动
#   - alun-kafka/rdkafka 消息队列（需 CMake + C 工具链）
#   - alun-task          Kafka 任务后端
#   - alun-fs / aws-sdk-s3  对象存储
HEAVY_RE='alun-db|alun-kafka|rdkafka|alun-task|alun-fs|aws-sdk-s3|aws-config|libsqlite3-sys'

fail=0

say()  { printf '%s\n' "$*"; }
pass() { printf '  \033[32mPASS\033[0m  %s\n' "$*"; }
bad()  { printf '  \033[31mFAIL\033[0m  %s\n' "$*"; fail=1; }

# tree <args...> —— 输出该 feature 组合下的依赖包名（每行一个）
tree() {
    cargo tree "$@" --edges normal --prefix none --no-dedupe 2>/dev/null \
        | sed 's/ v[0-9].*//' | sort -u
}

say ""
say "════════ Feature 纪律检查 ════════"

# ── C1：默认（core）必须是"干净"的 ────────────────────────────────
say ""
say "C1 · 默认 feature 集不含重量级依赖"
default_tree="$(tree -p alun)"
if [ -z "$default_tree" ]; then
    bad "无法获取 alun 默认依赖树（cargo tree 失败）"
else
    hit="$(printf '%s\n' "$default_tree" | grep -E "^($HEAVY_RE)" || true)"
    if [ -n "$hit" ]; then
        bad "默认依赖树中出现重量级依赖："
        printf '%s\n' "$hit" | sed 's/^/          /'
        say "          → 请把它们移到显式 feature 下（见 alun/Cargo.toml 分层注释）"
    else
        pass "默认（core）无 db / kafka / task / fs / 对象存储"
    fi
fi

# ── C2：默认必须真的包含 HTTP 能力（防止 core 被写空）─────────────
say ""
say "C2 · 默认 feature 集确实提供 HTTP 能力"
for pkg in alun-web alun-core axum tokio alun-config; do
    if printf '%s\n' "$default_tree" | grep -qx "$pkg"; then
        pass "默认包含 $pkg"
    else
        bad "默认缺少 $pkg —— core 可能被改成空集"
    fi
done

# ── C3：mini 预设 = SQLite，且仍不含 MySQL / Kafka / MinIO ────────
say ""
say "C3 · 预设 mini —— 自足但轻量"
mini_tree="$(tree -p alun --features mini)"
if [ -z "$mini_tree" ]; then
    bad "无法获取 mini 依赖树"
else
    printf '%s\n' "$mini_tree" | grep -q "alun-db-driver-sqlite" \
        && pass "mini 含 SQLite 驱动" \
        || bad "mini 缺少 SQLite 驱动（预设定义可能有误）"
    hit="$(printf '%s\n' "$mini_tree" | grep -E "^(alun-db-driver-mysql|alun-kafka|rdkafka|aws-sdk-s3)" || true)"
    [ -z "$hit" ] && pass "mini 不含 MySQL / Kafka / MinIO" \
                  || { bad "mini 混入了重型依赖："; printf '%s\n' "$hit" | sed 's/^/          /'; }
fi

# ── C4：lite 预设 = PostgreSQL + 缓存 + 插件 + 本地存储 ────────────
say ""
say "C4 · 预设 lite —— core + postgres + cache + plugin + fs-local"
lite_tree="$(tree -p alun --features lite)"
if [ -z "$lite_tree" ]; then
    bad "无法获取 lite 依赖树"
else
    printf '%s\n' "$lite_tree" | grep -q "alun-db-driver-postgres" \
        && pass "lite 含 PostgreSQL 驱动" \
        || bad "lite 缺少 PostgreSQL 驱动"
    for pkg in alun-cache alun-plugin alun-fs; do
        printf '%s\n' "$lite_tree" | grep -qx "$pkg" \
            && pass "lite 含 $pkg" \
            || bad "lite 缺少 $pkg —— 新定义是「PostgreSQL + 缓存 + 插件 + 本地存储」"
    done
    hit="$(printf '%s\n' "$lite_tree" | grep -E "^(alun-db-driver-mysql|alun-kafka|rdkafka|aws-sdk-s3|alun-template)" || true)"
    [ -z "$hit" ] && pass "lite 不含 MySQL / Kafka / MinIO / template" \
                  || { bad "lite 混入了不该有的依赖："; printf '%s\n' "$hit" | sed 's/^/          /'; }
fi

# ── C5：full 预设必须真的"全"（防止 feature 名写错而静默变空）──────
say ""
say "C5 · 预设 full —— 确实覆盖全部重型后端"
full_tree="$(tree -p alun --features full)"
if [ -z "$full_tree" ]; then
    bad "无法获取 full 依赖树"
else
    for pkg in alun-db-driver-mysql rdkafka aws-sdk-s3; do
        printf '%s\n' "$full_tree" | grep -qx "$pkg" \
            && pass "full 包含 $pkg" \
            || bad "full 缺少 $pkg —— 预设链可能已断（改名/拼写错误？）"
    done
    printf '%s\n' "$full_tree" | grep -qx "alun-template" \
        && pass "full 含 alun-template（template 不属于任何预设，须显式列入 full）" \
        || bad "full 缺少 alun-template —— 三个预设都不带模板了"
fi

# ── C6：`alun-core` 必须是干净的核心（零数据库 / 零 HTTP 依赖） ─────
# 反向守卫：曾经为了「DbError 能 `?` 进 alun::Result」把 `alun-db-core` 挂到
# `alun-core` 上；也曾让 `alun-core` 内嵌一整套 HTTP 响应类型（`Res` / `ApiError`）。
# 代价是核心层不再能被非 Web / 非 DB 项目单独复用。
# 正确做法：响应形状类型住在 `alun-web`，`ApiError` 成为本地类型后
# `From<任意外部错误>` 都合法，业务端只需 `ResResult<T>` 一个出口。
# 本断言防止有人以「方便」之名把数据库或 HTTP（或任何重型后端）重新拖进核心。
say ""
say "C6 · alun-core 依赖树不含数据库 / HTTP / 重型后端（核心层必须干净）"
core_tree="$(tree -p alun-core --all-features)"
if [ -z "$core_tree" ]; then
    bad "无法获取 alun-core 依赖树"
elif core_dirty="$(printf '%s\n' "$core_tree" | grep -Ex 'alun-db.*|alun-kafka|alun-task.*|alun-fs.*|rdkafka|axum.*|tower.*')"; then
    bad "alun-core 依赖了后端 crate：$(printf '%s' "$core_dirty" | tr '\n' ' ') —— 核心层不得知晓数据库 / HTTP 等后端，跨层转换请放 alun-web"
else
    pass "alun-core 保持零后端依赖（--all-features 下依然干净）"
fi

# ── C7：单一出口 —— prelude 必须有 Res/ResResult/ApiError，且不得再有 WebError ─────
# 业务端面对的类型越少越好：handler 唯一出口是 `ResResult<T>`。
# 曾经为此新造过 `WebError` / `WebResult`（那只是症状缓解），现已随
# `ApiError` 下沉 `alun-web` 而删除。本断言守住「不再长回第二个出口」。
say ""
say "C7 · 单一出口：prelude 导出 Res / ResResult / ApiError，且不含 WebError / WebResult"
prelude_src="$(sed -n '/^pub mod prelude {/,/^}/p' alun/src/lib.rs)"
missing=""
for sym in Res ResResult ApiError; do
    printf '%s\n' "$prelude_src" | grep -qw "$sym" \
        || missing="$missing $sym"
done
if [ -n "$missing" ]; then
    bad "prelude 缺少单一出口符号：$missing"
else
    pass "prelude 已导出单一出口 Res / ResResult / ApiError"
fi
if printf '%s\n' "$prelude_src" | grep -qwE "WebError|WebResult"; then
    bad "prelude 又出现了 WebError / WebResult —— 错误出口必须唯一，请只保留 ResResult"
else
    pass "prelude 不含第二出口（WebError / WebResult 已消失）"
fi

say ""
if [ "$fail" -eq 0 ]; then
    printf '\033[32m全部通过：feature 分层纪律完好。\033[0m\n'
else
    printf '\033[31m存在违规：默认依赖不得包含重型后端，能力必须按 feature 逐步叠加。\033[0m\n'
fi
say ""
exit "$fail"

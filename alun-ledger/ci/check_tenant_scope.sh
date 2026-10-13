#!/usr/bin/env bash
# 静态守卫（用例 TEN-005）：`src/api/**` 中所有列表/详情查询必须显式带租户作用域。
#
# 说明：这是一条**粗粒度文本检查**（框架无查询拦截器，见缺口 G2），
# 只能发现"明显没有 tenant 字样"的 SQL，不能替代运行时断言（TEN-002/TEN-007）。
set -uo pipefail
cd "$(dirname "$0")/.."

violations=0
while IFS= read -r file; do
  # 提取所有 SQL 字符串字面量中的 SELECT 语句所在行
  while IFS=: read -r line_no text; do
    case "$text" in
      *SELECT*|*"DELETE FROM"*|*"UPDATE "*)
        # 白名单：
        #  - 明确标注为跨租户/管理员的查询（`tenant_id` / `cross_tenant` / `admin_*`）
        #  - 健康探针 `SELECT 1`（不读取任何租户表，见 api/subscriptions.rs 的 /health）
        case "$text" in
          *tenant_id*|*cross_tenant*|*admin_*|*"SELECT 1"*) ;;
          *)
            echo "❌ $file:$line_no 缺少租户条件: $text"
            violations=$((violations + 1))
            ;;
        esac
        ;;
    esac
  done < <(grep -n 'SELECT\|DELETE FROM\|UPDATE ' "$file" 2>/dev/null || true)
done < <(find src/api -name '*.rs')

if [ "$violations" -ne 0 ]; then
  echo "共 $violations 处可能越租户的查询（请补 tenant_id 或标注 cross_tenant）"
  exit 1
fi
echo "✅ src/api/** 的查询均显式带租户条件"

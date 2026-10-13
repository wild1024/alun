#!/usr/bin/env bash
# 红线门禁（《测试项目测试规划.md》§11 的自动化映射）。
#
# 任一步失败 → 总分判定不及格。
# 用法：bash ci/redline.sh [sqlite-mem|sqlite-file|postgres|mysql]
set -uo pipefail

DIALECT="${1:-sqlite-mem}"
export ALUN_LEDGER_DB="$DIALECT"

case "$DIALECT" in
  sqlite-mem|sqlite-file) FEATURES="sqlite" ;;
  postgres)               FEATURES="postgres" ;;
  mysql)                  FEATURES="mysql" ;;
  *) echo "未知方言: $DIALECT" >&2; exit 2 ;;
esac

fail=0
run() {
  local redline="$1"; shift
  echo "── 红线: $redline ($DIALECT)"
  if "$@"; then
    echo "   ✅ 通过"
  else
    echo "   ❌ 失败"
    fail=1
  fi
}

# 1) Decimal 精度 / 读回（判定以 PG 为准；SQLite 文本承载差异即 D001，已修复，此项转为回归门禁）
run "Decimal 精度与读回" \
  cargo test -p alun-ledger --features "$FEATURES" --test ledger_tx lgr_011 -- --nocapture
run "Decimal 聚合/排序探针" \
  cargo test -p alun-ledger --features "$FEATURES" --test adversarial_types typ_00 -- --nocapture

# 2) 事务原子性
run "事务原子性（失败即整体回滚）" \
  cargo test -p alun-ledger --features "$FEATURES" --test ledger_tx lgr_00 -- --nocapture

# 3) 并发不产生负数余额
run "并发无负余额" \
  cargo test -p alun-ledger --features "$FEATURES" --test adversarial_concurrency -- --nocapture

# 4) 迁移可回滚
run "迁移回滚" \
  cargo test -p alun-ledger --features "$FEATURES" --test migration_evolution mig_002 -- --nocapture

# 5) SQL 注入拒绝
run "SQL 注入回归" \
  cargo test -p alun-ledger --features "$FEATURES" --test adversarial_query qry_005 -- --nocapture

# 6) 静默错值回归（Decimal/数组/bool 读回；三类缺陷 D001/D009/D021 均已修复，此处防止回退）
run "静默错值探针" \
  cargo test -p alun-ledger --features "$FEATURES" --test adversarial_types -- --nocapture

# 7) 连接泄漏（需要 pool_stats，缺口 M-3 → 退化为长事务不挂死）
run "长事务不耗尽连接池" \
  cargo test -p alun-ledger --features "$FEATURES" --test adversarial_concurrency -- --nocapture

if [ "$fail" -ne 0 ]; then
  echo
  echo "红线门禁未通过（见上方 ❌ 项）"
  exit 1
fi
echo
echo "全部红线通过 ✅"

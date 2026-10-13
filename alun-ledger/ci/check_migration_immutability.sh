#!/usr/bin/env bash
# 迁移不可变守卫（规划 §12 D-007 的项目侧缓解）：
# 已发布的迁移文件一旦被改动，校验和即不匹配 → CI 失败。
#
# 框架现状（已按缺陷修复轮更新）：`schema_migrations(version, name, applied_at, checksum, applied_seq)`
# **已内置 checksum 校验**（`alun-db/src/migrate.rs`，缺陷 D011 已修复）与并发咨询锁，启动即拒绝
# 被改动的已应用迁移；本 CI 守卫作为**第二道防线**保留：在提交期（迁移尚未运行）即发现已发布迁移被改动。
#
# 用法：
#   bash ci/check_migration_immutability.sh          # 校验（CI 用）
#   bash ci/check_migration_immutability.sh --init   # 生成/更新清单（新增迁移后执行）
set -uo pipefail
cd "$(dirname "$0")/.."

MANIFEST="migrations/.immutability.sha256"
hash_of() { if command -v shasum >/dev/null 2>&1; then shasum -a 256 "$1" | awk '{print $1}'; else sha256sum "$1" | awk '{print $1}'; fi; }

current() {
  find migrations -name '*.sql' | LC_ALL=C sort | while IFS= read -r f; do
    printf '%s  %s\n' "$(hash_of "$f")" "$f"
  done
}

if [ "${1:-}" = "--init" ]; then
  current > "$MANIFEST"
  echo "✅ 已写入 ${MANIFEST}（$(wc -l < "$MANIFEST" | tr -d ' ') 个文件）"
  exit 0
fi

[ -f "$MANIFEST" ] || { echo "❌ 缺少 ${MANIFEST}（先执行 --init）"; exit 1; }

violations=0
while IFS= read -r line; do
  [ -z "$line" ] && continue
  want="${line%%  *}"; file="${line##*  }"
  if [ ! -f "$file" ]; then
    echo "❌ 迁移文件被删除: $file"; violations=$((violations + 1)); continue
  fi
  got="$(hash_of "$file")"
  [ "$want" = "$got" ] || { echo "❌ 迁移文件被篡改: $file"; violations=$((violations + 1)); }
done < "$MANIFEST"

# 新增但未登记的文件（提示，不失败：新增迁移属正常演进）
while IFS= read -r f; do
  grep -q "  $f\$" "$MANIFEST" || echo "ℹ️  新增迁移未登记（发布前执行 --init）: $f"
done < <(find migrations -name '*.sql' | LC_ALL=C sort)

if [ "$violations" -ne 0 ]; then
  echo "共 $violations 处迁移不可变违规"
  exit 1
fi
echo "✅ 已发布迁移未改动（$(wc -l < "$MANIFEST" | tr -d ' ') 个文件校验和一致）"

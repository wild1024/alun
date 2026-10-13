#!/usr/bin/env bash
# 缺陷收集（《测试项目测试规划.md》§12）：校验"每个 ❌/🔶W 用例都有对应缺陷条目"。
#
# 用法：bash ci/collect_defects.sh
set -uo pipefail
cd "$(dirname "$0")/../.."

PLAN="测试项目测试规划.md"

# 只对「规划里标为未满足（W / [FAIL]）」的用例要求缺陷登记（规划 R2：
# 能力缺口必须显式登记，不得用 raw SQL 悄悄绕过）。✅E 的用例不要求条目。
[ -f "$PLAN" ] || { echo "❌ 找不到 ${PLAN}（脚本需在仓库根目录下的 alun-ledger/ci 内运行）"; exit 2; }
[ -f alun-ledger/DEFECTS.md ] || { echo "❌ 找不到 alun-ledger/DEFECTS.md"; exit 2; }
UNMET="$(
  grep -oE '^\| (LGR|INV|SUB|TEN|AUD|RPT|REC|MIG|API|OBS|TYP|QRY|ERR)-[0-9]{3} \|.*' "$PLAN" \
  | awk -F'|' '{ id=$2; gsub(/ /,"",id); st=$(NF-1); gsub(/^ +| +$/,"",st);
                 if (st ~ /❌/ || st ~ /⚠/ || st ~ /^W/ || st ~ /FAIL/) print id }' \
  | sort -u
)"
DEFECTS="$(cat alun-ledger/DEFECTS.md)"

missing=0
while IFS= read -r case_id; do
  [ -z "$case_id" ] && continue
  # DEFECTS.md 中用例以测试函数名的形式引用（`typ_003`），此处归一化大小写与分隔符
  key="$(printf '%s' "$case_id" | tr 'A-Z-' 'a-z_')"
  if ! grep -qi "$case_id" <<<"$DEFECTS" && ! grep -qi "$key" <<<"$DEFECTS"; then
    echo "❌ 用例 $case_id 标为未满足，但 DEFECTS.md 未登记"
    missing=$((missing + 1))
  fi
done <<<"$UNMET"

echo
echo "缺陷条目数: $(grep -ohE '^\| D[0-9]{3} ' alun-ledger/DEFECTS.md | sort -u | wc -l | tr -d ' ')"
if [ "$missing" -ne 0 ]; then echo "共 ${missing} 个未满足用例缺少缺陷登记"; exit 1; fi
echo "未满足用例均已登记缺陷 ✅（规划 R2）"

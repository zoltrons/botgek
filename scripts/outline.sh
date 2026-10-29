#!/usr/bin/env bash
# Ajanlar ve geliştiriciler için hızlı kod gezinme aracı. Büyük dosyayı baştan sona okumak yerine
# önce çatısına bakılır, sonra yalnızca gereken aralık okunur.
#
#   scripts/outline.sh --tree            # her kaynak dosya: satır sayısı + ilk //! satırı
#   scripts/outline.sh FILE|DIR ...      # öğe çatısı (fn/struct/enum/...) satır numaralarıyla
#   scripts/outline.sh --find ADI        # bir tanımın nerede olduğu (fn/struct/enum/trait/const/...)
#   scripts/outline.sh --refs ADI        # ADI'nın kullanıldığı dosyalar ve sayıları
#   scripts/outline.sh --check           # docs/agents/MAP.md depoyla ayrışmış mı
#
# Çıktı her zaman çalışma ağacının o anki hâlidir; satır numaraları bu yüzden güvenilirdir ve
# hiçbir yere kaydedilmez.

set -euo pipefail
cd "$(dirname "$0")/.."

# `cut -c` Türkçe karakteri ortadan bölmesin diye UTF-8 yerel ayarı seçilir.
for loc in C.UTF-8 C.utf8 en_US.UTF-8; do
  if locale -a 2>/dev/null | grep -qix "$loc"; then export LC_ALL=$loc; break; fi
done

MAP=docs/agents/MAP.md
ITEM='^[[:space:]]{0,4}(pub(\([a-z]+\))? )?(async )?(unsafe )?(fn|struct|enum|trait|type|const|static|mod|impl|macro_rules!) '

src_files() { git ls-files --cached --others --exclude-standard 'src/*' | grep -E '\.(rs|js|css|html)$'; }

# Test modülleri (`#[cfg(test)] mod ... { }`) atlanır, yalnızca konumları ve test sayıları yazılır;
# dosya ortasındaki test modülünden sonra gelen üretim kodu da çatıya girer.
outline_rs() {
  local f=$1
  echo "== $f ($(wc -l <"$f" | tr -d ' ') satır)"
  awk '
    /^#\[cfg\(test\)\]/ { pend = 1; next }
    pend && /^mod [A-Za-z_0-9]+ *\{/ { intest = 1; start = NR; nt = 0; pend = 0; next }
    { pend = 0 }
    intest {
      if ($0 ~ /^[[:space:]]*#\[(tokio::)?test\]/) nt++
      if ($0 ~ /^\}/) { intest = 0; printf "@testler %d-%d (%d test)\n", start, NR, nt }
      next
    }
    { printf "%d:%s\n", NR, $0 }
  ' "$f" |
    grep -E "^@testler|^[0-9]+:${ITEM#^}" |
    sed -E 's/[[:space:]]*\{[[:space:]]*$//; s/[[:space:]]+/ /g' | cut -c1-110 || true
}

outline_js() {
  local f=$1
  echo "== $f ($(wc -l <"$f" | tr -d ' ') satır)"
  grep -nE '^(async )?function |^(const|let) [A-Za-z_$]+ = (async )?(\(|function)|^// ──' "$f" |
    sed -E 's/[[:space:]]*\{[[:space:]]*$//' | cut -c1-110
}

outline_other() {
  local f=$1
  echo "== $f ($(wc -l <"$f" | tr -d ' ') satır)"
  grep -nE '^(/\* ──|<!-- ──|  <(section|template) |<section )' "$f" | cut -c1-110 || true
}

outline() {
  local target=$1 f
  if [ -d "$target" ]; then
    git ls-files "$target" | grep -E '\.(rs|js)$' | while read -r f; do outline "$f"; done
    return
  fi
  [ -f "$target" ] || { echo "yok: $target" >&2; return 1; }
  case "$target" in
    *.rs) outline_rs "$target" ;;
    *.js) outline_js "$target" ;;
    *) outline_other "$target" ;;
  esac
}

tree() {
  local f lines doc
  src_files | while read -r f; do
    lines=$(wc -l <"$f" | tr -d ' ')
    doc=$(grep -m1 -E '^//! ' "$f" 2>/dev/null | sed 's|^//! ||' | cut -c1-80 || true)
    printf '%5s  %-46s %s\n' "$lines" "$f" "$doc"
  done
}

find_def() {
  local name=$1
  git grep -nE "^[[:space:]]*(pub(\([a-z]+\))? )?(async )?(unsafe )?(fn|struct|enum|trait|type|const|static|mod|macro_rules!) ${name}([^A-Za-z0-9_]|$)" -- 'src/*.rs' |
    cut -c1-140
  git grep -nE "^(async )?function ${name}([^A-Za-z0-9_]|$)|^(const|let) ${name} = " -- 'src/web/ui/*.js' | cut -c1-140 || true
}

refs() {
  git grep -cwF -- "$1" -- 'src/*' 'tests/*' 'docs/*' | sed 's/:/  /' | sort -t' ' -k3 -nr
}

check() {
  local bad=0 f p
  [ -f "$MAP" ] || { echo "$MAP yok" >&2; exit 1; }
  # 1) Her kaynak dosya haritada geçmeli.
  for f in $(git ls-files --cached --others --exclude-standard 'src/*.rs' 'tests/*.cjs'); do
    if ! grep -qF "$f" "$MAP"; then
      echo "haritada yok: $f"
      bad=1
    fi
  done
  # 2) Her modül belgesi modül dizininde (docs/modules/README.md) listelenmeli; harita oraya yönlendirir.
  for f in $(git ls-files --cached --others --exclude-standard 'docs/modules/*.md' 'docs/modules/platforms/*.md'); do
    [ "$f" = docs/modules/README.md ] && continue
    if ! grep -qF "${f#docs/modules/}" docs/modules/README.md; then
      echo "modül dizininde yok: $f"
      bad=1
    fi
  done
  # 3) Haritadaki her yol var olmalı.
  for p in $(grep -oE '`(src|docs|tests|scripts)/[A-Za-z0-9_./-]+`' "$MAP" | tr -d '`' | sort -u); do
    if [ ! -e "$p" ] && ! git ls-files --error-unmatch "$p" >/dev/null 2>&1; then
      echo "haritada olan yol yok: $p"
      bad=1
    fi
  done
  # 4) Haritadaki çıplak belge adları (converters.md gibi) docs/ altında bulunmalı.
  for p in $(grep -oE '[A-Za-z_-]+\.md' "$MAP" | sort -u); do
    if ! git ls-files --cached --others --exclude-standard 'docs/*' '*.md' | grep -qE "(^|/)${p}$"; then
      echo "haritadaki belge yok: $p"
      bad=1
    fi
  done
  if [ "$bad" -eq 0 ]; then
    echo "harita güncel"
  else
    echo "-> $MAP dosyasını güncelleyin" >&2
    exit 1
  fi
}

case "${1:-}" in
  --tree) tree ;;
  --find) [ -n "${2:-}" ] || { echo "kullanım: --find ADI" >&2; exit 2; }; find_def "$2" ;;
  --refs) [ -n "${2:-}" ] || { echo "kullanım: --refs ADI" >&2; exit 2; }; refs "$2" ;;
  --check) check ;;
  ""|-h|--help) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//' ;;
  *) for t in "$@"; do outline "$t"; done ;;
esac

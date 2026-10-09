#!/usr/bin/env sh
# Botgek sürüm yayınlama — sürümü bump'lar, `Cargo.lock`'u senkronlar, commit + **annotated
# tag** atar ve (istenirse) push'lar. Etiket push'u CI'yı tetikler: testler → `.deb`/`.tar.gz`
# → GitHub Release → multi-arch Docker imajı (bkz. `.github/workflows/build-release.yml`).
#
# Kullanım:
#   ./release.sh patch|minor|major        # 0.1.0 → 0.1.1 / 0.2.0 / 1.0.0
#   ./release.sh 1.2.0                    # açık sürüm
#   ./release.sh minor --push             # push'la (CI yayını başlar)
#   ./release.sh patch --no-test          # testleri atla (önerilmez)
#
# Sözleşme:
#   * Sürümün **tek kaynağı** `Cargo.toml` (`[package] version`) + `vX.Y.Z` etiketidir.
#     Derleme sırasında `build.rs` tam künyeyi etiketten türetir (`0.2.0+5.g8fc2692`).
#   * Çalışma ağacı **temiz** olmalıdır; aksi hâlde yayın commit'i ilgisiz değişiklikleri alır.
#   * Ön sürüm (`1.0.0-rc.1`) artıştan önce atılır (semver: `rc` → `release`).
set -eu

cd "$(git rev-parse --show-toplevel)"

bump=""
push=0
run_tests=1

for arg in "$@"; do
    case "$arg" in
        --push) push=1 ;;
        --no-test) run_tests=0 ;;
        -h|--help)
            sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
            exit 0
            ;;
        -*) echo "bilinmeyen seçenek: $arg" >&2; exit 2 ;;
        *)
            if [ -n "$bump" ]; then echo "fazla argüman: $arg" >&2; exit 2; fi
            bump="$arg"
            ;;
    esac
done

if [ -z "$bump" ]; then
    echo "kullanım: ./release.sh patch|minor|major|<x.y.z> [--push] [--no-test]" >&2
    exit 2
fi

if [ -n "$(git status --porcelain)" ]; then
    echo "hata: çalışma ağacı temiz değil — önce commit edin (yayın commit'i kirli olmasın)" >&2
    git status --short >&2
    exit 1
fi

# `[package]` bölümündeki sürüm (bağımlılık `version =` satırlarına dokunmaz).
current="$(awk -F'"' '
    /^\[package\]/ { in_pkg = 1; next }
    /^\[/ { in_pkg = 0 }
    in_pkg && !found && /^version[[:space:]]*=/ { print $2; found = 1 }
' Cargo.toml)"

if [ -z "$current" ]; then
    echo "hata: Cargo.toml içinde [package] sürümü bulunamadı" >&2
    exit 1
fi

# Ön sürümü at: 1.0.0-rc.1 → 1.0.0 (artış tabandan yapılır).
base="${current%%-*}"
major="${base%%.*}"
rest="${base#*.}"
minor="${rest%%.*}"
patch="${rest#*.}"

case "$bump" in
    major) next="$((major + 1)).0.0" ;;
    minor) next="$major.$((minor + 1)).0" ;;
    patch) next="$major.$minor.$((patch + 1))" ;;
    *) next="$bump" ;;
esac

if ! printf '%s' "$next" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$'; then
    echo "hata: geçersiz sürüm: $next" >&2
    exit 2
fi

if git rev-parse -q --verify "refs/tags/v$next" >/dev/null 2>&1; then
    echo "hata: v$next etiketi zaten var" >&2
    exit 1
fi

echo "sürüm: $current → $next"

# 1) `Cargo.toml` [package] sürümü (yalnızca o satır).
tmp="$(mktemp)"
awk -v next="$next" '
    /^\[package\]/ { in_pkg = 1; print; next }
    /^\[/ { in_pkg = 0 }
    in_pkg && !done && /^version[[:space:]]*=/ { print "version = \"" next "\""; done = 1; next }
    { print }
' Cargo.toml > "$tmp"
mv "$tmp" Cargo.toml
grep -q "^version = \"$next\"$" Cargo.toml || { echo "hata: Cargo.toml güncellenemedi" >&2; exit 1; }

# 2) `Cargo.lock` senkronu: `--locked` derlemeler sürüm ayrışırsa başarısız olur.
if command -v cargo >/dev/null 2>&1; then
    cargo update --package botgek --precise "$next" >/dev/null
else
    echo "uyarı: cargo bulunamadı — Cargo.lock elle güncellenmeli (--locked derlemeler kırılır)" >&2
fi

# 3) Testler (varsayılan): kırık kod etiketlenmesin.
if [ "$run_tests" = 1 ]; then
    if command -v cargo >/dev/null 2>&1; then
        echo "testler çalışıyor (cargo test)..."
        cargo test --quiet
    else
        echo "uyarı: cargo bulunamadı — testler atlandı" >&2
    fi
fi

# 4) Commit + annotated etiket.
git add Cargo.toml
[ -f Cargo.lock ] && git add Cargo.lock
git commit -q -m "chore(release): v$next" -m "Sürüm kaynağı: Cargo.toml + v$next etiketi; build.rs tam künyeyi etiketten türetir."
git tag -a "v$next" -m "Botgek v$next"

# 5) Push (isteğe bağlı): etiket push'u CI yayın zincirini tetikler.
if [ "$push" = 1 ]; then
    branch="$(git rev-parse --abbrev-ref HEAD)"
    git push origin "HEAD:refs/heads/$branch"
    git push origin "v$next"
    echo "push edildi: $branch + v$next → CI yayını tetiklendi"
else
    echo "yerel hazır. Yayınlamak için: git push origin HEAD && git push origin v$next"
fi

echo "doğrulama: cargo run -- --version"

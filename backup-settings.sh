#!/usr/bin/env bash
# @file backup-settings.sh
# @brief Botgek ayar yedeği — **göç öncesi** mevcut ayar dosyasının zaman damgalı kopyasını alır.
# @description
#   Kullanım: `./backup-settings.sh [settings.json yolu]`
#
#   Yol verilmezse botun kendi çözüm sırası izlenir:
#   - `BOTGEK_SETTINGS_PATH`,
#   - `BOTGEK_DB_PATH` bir dizin içeriyorsa o dizindeki `settings.json`,
#   - `settings.json` ve `settings.json/settings.json` (Docker bind-mount'un klasör olarak açıldığı biçim).
#
#   Kurulum türüne göre dosya:
#   - systemd/`.deb`: `/var/lib/botgek/settings.json` → `sudo ./backup-settings.sh /var/lib/botgek/settings.json`
#   - Docker Compose: volume içindeki `/app/data/settings.json` → `docker compose exec botgek cat /app/data/settings.json > settings.docker.json`, ardından `./backup-settings.sh settings.docker.json` (hosttaki `settings.json` ilk içe aktarım kaynağıdır, üzerine yazmayın).
#
#   Yedek adı `settings.json.bak-YYYYMMDD-HHMMSS`; aynı saniyede ad varsa `-1`, `-2`… eklenir, mevcut yedeğin üzerine asla yazılmaz.
#   Kopya önce aynı dizinde geçici dosyaya alınır, doğrulanır ve atomik olarak (hard link, var olan adı ezmez) yerine konur.
#
#   Neden gerekli: sunucu ayarları göçü (tam kopya → bölüm bazlı sparse override) yükleme anında **bir kez** çalışır ve eşit bölümleri mirasa döndürür.
#   Bu dönüşüm geri alınamaz, bu yüzden göç öncesi durum fotoğrafı şarttır.
#
#   Geri alma: yedeği `settings.json` üzerine kopyalayın ve botu **eski sürümle** yeniden başlatın (eski sürüm tam kopya kayıtları bekler).
#
#   Yeni sürümde bot, göçü uygulamadan **önce** aynı yedeği kendi de alır (`settings.json.bak-<unix saniye>`); bu script sürüm yükseltmeden önce elle kontrol edebilmeniz için vardır.
set -euo pipefail

tmp=""
backup=""

# @description Hata ya da kesinti durumunda geçici kopyayı siler.
# @noargs
cleanup() {
    if [ -n "$tmp" ]; then
        rm -f -- "$tmp"
    fi
}
trap cleanup EXIT

# @description Ayar dosyasının yolunu botla aynı sırayla bulur.
# @arg $1 string Açıkça verilen yol (boş olabilir).
# @stdout Bulunan yol; bulunamazsa boş.
resolve_target() {
    local explicit="$1"
    if [ -n "$explicit" ]; then
        printf '%s\n' "$explicit"
        return
    fi
    if [ -n "${BOTGEK_SETTINGS_PATH:-}" ]; then
        printf '%s\n' "$BOTGEK_SETTINGS_PATH"
        return
    fi
    local db="${BOTGEK_DB_PATH:-}"
    if [ -n "$db" ] && [ "$db" != ":memory:" ] && [ "${db%/*}" != "$db" ]; then
        local candidate="${db%/*}/settings.json"
        if [ -f "$candidate" ]; then
            printf '%s\n' "$candidate"
            return
        fi
    fi
    local candidate
    for candidate in "settings.json" "settings.json/settings.json"; do
        if [ -f "$candidate" ]; then
            printf '%s\n' "$candidate"
            return
        fi
    done
}

# @description Zaman damgalı, var olan hiçbir dosyayı ezmeyen yedeği atomik olarak oluşturur.
# @arg $1 string Yedeklenecek dosya.
# @set backup string Oluşan yedeğin yolu.
# @exitcode 1 Kopya doğrulanamazsa veya uygun ad bulunamazsa.
make_backup() {
    local source="$1"
    local dir base stamp n
    dir="$(dirname -- "$source")"
    base="$(basename -- "$source")"
    stamp="$(date +%Y%m%d-%H%M%S)"

    tmp="$(mktemp "$dir/.${base}.bak-tmp.XXXXXX")"
    cp -p -- "$source" "$tmp"
    if ! cmp -s -- "$source" "$tmp"; then
        echo "hata: geçici kopya doğrulanamadı ('$tmp')" >&2
        return 1
    fi

    backup="$source.bak-$stamp"
    n=0
    # `ln` hedef varsa başarısız olur: ad seçimi ile yerleştirme tek atomik adımdır, yarışta bile üzerine yazılmaz.
    until ln -- "$tmp" "$backup" 2>/dev/null; do
        n=$((n + 1))
        if [ "$n" -gt 100 ]; then
            echo "hata: '$source.bak-$stamp' için boş yedek adı bulunamadı" >&2
            return 1
        fi
        backup="$source.bak-$stamp-$n"
    done
    rm -f -- "$tmp"
    tmp=""
}

# @description Yedeğin şema sürümünü ve sunucu kayıtlarının tuttuğu bölümleri yazdırır.
# @arg $1 string Yedek dosyası.
# @stdout Özet satırları.
print_summary() {
    command -v python3 >/dev/null 2>&1 || return 0
    python3 - "$1" <<'PY' || true
import json, sys
with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
guilds = data.get("guilds") or {}
print("şema sürümü: {} · sunucu kaydı: {}".format(data.get("schema_version", 0), len(guilds)))
blocks = ("modules", "role_filters", "ai", "engine")
for guild_id, record in guilds.items():
    if not isinstance(record, dict):
        print("  {}: (bozuk kayıt)".format(guild_id))
        continue
    pinned = [block for block in blocks if record.get(block) is not None]
    print("  {}: {}".format(guild_id, ", ".join(pinned) if pinned else "—"))
PY
}

# @description Ayar dosyasını bulur, doğrular, yedekler ve özetler.
# @arg $1 string İsteğe bağlı ayar dosyası yolu.
# @exitcode 1 Dosya bulunamazsa veya yedek alınamazsa.
main() {
    local target
    target="$(resolve_target "${1:-}")"

    if [ -z "$target" ] || [ ! -f "$target" ]; then
        local shown=""
        if [ -n "$target" ]; then shown=" ('$target')"; fi
        echo "hata: ayar dosyası bulunamadı$shown (yolu argüman olarak verin)" >&2
        echo "  systemd: sudo $0 /var/lib/botgek/settings.json" >&2
        echo "  Docker:  docker compose exec botgek cat /app/data/settings.json > settings.docker.json && $0 settings.docker.json" >&2
        exit 1
    fi

    if command -v python3 >/dev/null 2>&1; then
        python3 -c 'import json,sys; json.load(open(sys.argv[1], encoding="utf-8"))' "$target" 2>/dev/null \
            || echo "uyarı: '$target' geçerli JSON olarak okunamadı (yine de yedekleniyor)" >&2
    fi

    make_backup "$target"
    print_summary "$backup"
    echo "yedek alındı: $backup"
}

main "$@"

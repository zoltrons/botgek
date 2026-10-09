#!/usr/bin/env sh
# Botgek ayar yedeği — **göç öncesi** mevcut ayar dosyasının zaman damgalı kopyasını alır.
#
# Kullanım:
#   ./backup-settings.sh [settings.json yolu]
#
# Yol verilmezse sırayla `settings.json` ve `settings.json/settings.json` (Docker
# bind-mount'un klasör olarak açıldığı biçim) denenir. Yedek adı: `settings.json.bak-YYYYMMDD-HHMMSS`
#
# Neden gerekli: sunucu ayarları göçü (tam kopya → bölüm bazlı sparse override) yükleme
# anında **bir kez** çalışır ve eşit bölümleri mirasa döndürür. Bu dönüşüm geri alınamaz,
# bu yüzden göç öncesi durum fotoğrafı şarttır.
#
# Geri alma: yedeği `settings.json` üzerine kopyalayın ve botu **eski sürümle** yeniden
# başlatın (eski sürüm tam kopya kayıtları bekler).
#
# Yeni sürümde bot, göçü uygulamadan **önce** aynı yedeği kendi de alır
# (`settings.json.bak-<unix saniye>`); bu script sürüm yükseltmeden önce elle kontrol
# edebilmeniz için vardır.
set -eu

target="${1:-}"
if [ -z "$target" ]; then
    for candidate in "settings.json" "settings.json/settings.json"; do
        if [ -f "$candidate" ]; then
            target="$candidate"
            break
        fi
    done
fi

if [ -z "$target" ] || [ ! -f "$target" ]; then
    echo "hata: ayar dosyası bulunamadı (yolu argüman olarak verin)" >&2
    exit 1
fi

if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; json.load(open(sys.argv[1], encoding="utf-8"))' "$target" 2>/dev/null \
        || echo "uyarı: '$target' geçerli JSON olarak okunamadı (yine de yedekleniyor)" >&2
fi

stamp="$(date +%Y%m%d-%H%M%S)"
backup="${target}.bak-${stamp}"
cp -p "$target" "$backup"

if ! cmp -s "$target" "$backup"; then
    echo "hata: yedek doğrulanamadı ('$backup')" >&2
    exit 1
fi

# Göç öncesi durum fotoğrafı: şema sürümü ve sunucu kayıtlarının hangi bölümleri tuttuğu.
if command -v python3 >/dev/null 2>&1; then
    python3 - "$backup" <<'PY' || true
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
fi

echo "yedek alındı: $backup"

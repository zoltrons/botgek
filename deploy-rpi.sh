#!/usr/bin/env bash
# ==============================================================================
# Botgek - Raspberry Pi Debian Hızlı Kurulum Betiği
# ==============================================================================
set -e

echo "=== Botgek Raspberry Pi Debian Kurulumu Başlatılıyor ==="

# 1. Root / Sudo kontrolü
if [ "$EUID" -ne 0 ]; then
  echo "[!] Lütfen bu betiği root veya sudo yetkisiyle çalıştırın: sudo bash deploy-rpi.sh"
  exit 1
fi

# 2. Gerekli sistem paketlerini kur
echo "[*] Bağımlılıklar kontrol ediliyor..."
apt-get update -qq
apt-get install -y -qq ca-certificates curl systemd tzdata

# 3. botgek sistem kullanıcısını oluştur
if ! id -u botgek &>/dev/null; then
    echo "[*] 'botgek' sistem kullanıcısı oluşturuluyor..."
    useradd -r -s /bin/false -d /var/lib/botgek botgek
fi

# 4. Çalışma ve yapılandırma dizinlerini hazırla
echo "[*] Dizinler oluşturuluyor..."
mkdir -p /etc/botgek
mkdir -p /var/lib/botgek

# 5. İkili dosyayı (Binary) kur
if [ -f "./botgek" ]; then
    echo "[*] 'botgek' ikili dosyası /usr/bin/ dizinine kopyalanıyor..."
    cp ./botgek /usr/bin/botgek
    chmod +x /usr/bin/botgek
else
    echo "[!] './botgek' ikili dosyası bulunamadı! Lütfen arşivin içindeki dizinden çalıştırın."
    exit 1
fi

# 6. Ortam değişkenleri şablonunu kopyala (varsa üzerine yazma)
if [ ! -f "/etc/botgek/botgek.env" ]; then
    if [ -f "./.env.example" ]; then
        cp ./.env.example /etc/botgek/botgek.env
        chmod 600 /etc/botgek/botgek.env
        chown botgek:botgek /etc/botgek/botgek.env
        echo "[*] /etc/botgek/botgek.env oluşturuldu. Lütfen Discord Token ve API anahtarınızı girin!"
    fi
else
    echo "[*] /etc/botgek/botgek.env zaten mevcut, mevcut dosya korundu."
fi

# 7. Ayarlar dosyasını çalışma dizinine kopyala
if [ -f "./settings.json" ] && [ ! -f "/var/lib/botgek/settings.json" ]; then
    cp ./settings.json /var/lib/botgek/settings.json
fi
chown -R botgek:botgek /var/lib/botgek

# 8. Systemd servisini kur ve etkinleştir
if [ -f "./botgek.service" ]; then
    echo "[*] Systemd servisi yapılandırılıyor..."
    cp ./botgek.service /etc/systemd/system/botgek.service
    systemctl daemon-reload
    systemctl enable botgek.service
fi

echo "================================================================="
echo " [OK] Botgek kurulumu tamamlandı!"
echo ""
echo " Sonraki Adımlar:"
echo " 1. Ayar dosyanızı düzenleyin:"
echo "    sudo nano /etc/botgek/botgek.env"
echo ""
echo " 2. Botu başlatın:"
echo "    sudo systemctl start botgek"
echo ""
echo " 3. Bot durumunu ve logları takip edin:"
echo "    sudo systemctl status botgek"
echo "    sudo journalctl -u botgek -f"
echo ""
echo " 4. WebUI Kontrol Paneline erişin:"
echo "    http://<raspberry_pi_ip_adresi>:3000"
echo "================================================================="

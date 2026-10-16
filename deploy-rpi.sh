#!/usr/bin/env bash
# @file deploy-rpi.sh
# @brief Botgek'i tarball içeriğinden Raspberry Pi / Debian'a kurar veya günceller.
# @description
#   Arşivin açıldığı dizinden root olarak çalıştırılır: `sudo bash deploy-rpi.sh`.
#   Kullanıcıyı, dizinleri, ikiliyi, ortam dosyasını ve systemd birimini hazırlar.
#   Güncellemede çalışan servis yeni ikiliyle yeniden başlatılır; ilk kurulumda servis yalnızca etkinleştirilir (token girilmeden başlatılmaz).
set -euo pipefail

ENV_DIR=/etc/botgek
ENV_FILE="$ENV_DIR/botgek.env"
STATE_DIR=/var/lib/botgek
BIN_PATH=/usr/bin/botgek
UNIT_PATH=/etc/systemd/system/botgek.service

# @description Betiğin root olarak çalıştığını doğrular.
# @noargs
# @exitcode 1 root değilse.
require_root() {
    if [ "$EUID" -ne 0 ]; then
        echo "[!] Lütfen bu betiği root veya sudo yetkisiyle çalıştırın: sudo bash deploy-rpi.sh" >&2
        exit 1
    fi
}

# @description Çalışma zamanı bağımlılıklarını kurar.
# @noargs
install_dependencies() {
    echo "[*] Bağımlılıklar kontrol ediliyor..."
    apt-get update -qq
    apt-get install -y -qq ca-certificates curl systemd tzdata
}

# @description `botgek` sistem grubunu ve kullanıcısını yoksa oluşturur (`.deb` paketinin `postinst`'iyle aynı biçim).
# @noargs
ensure_account() {
    if ! getent group botgek >/dev/null; then
        groupadd --system botgek
    fi
    if ! id -u botgek >/dev/null 2>&1; then
        echo "[*] 'botgek' sistem kullanıcısı oluşturuluyor..."
        useradd --system --gid botgek --home-dir "$STATE_DIR" --no-create-home --shell /usr/sbin/nologin botgek
    fi
}

# @description Yapılandırma ve kalıcı veri dizinlerini hazırlar.
# @noargs
prepare_directories() {
    echo "[*] Dizinler oluşturuluyor..."
    install -d -m 0755 -o root -g root "$ENV_DIR"
    install -d -m 0750 -o botgek -g botgek "$STATE_DIR"
}

# @description İkiliyi yanına geçici adla kurup atomik olarak yerine taşır.
# Çalışan bir ikilinin üzerine `cp` yazmak `ETXTBSY` ile başarısız olur; `rename` ise eski inode'u çalışan süreçte bırakır.
# @noargs
# @exitcode 1 `./botgek` bulunamazsa.
install_binary() {
    if [ ! -f ./botgek ]; then
        echo "[!] './botgek' ikili dosyası bulunamadı! Lütfen arşivin içindeki dizinden çalıştırın." >&2
        exit 1
    fi
    echo "[*] 'botgek' ikili dosyası $BIN_PATH konumuna kuruluyor..."
    install -m 0755 -o root -g root ./botgek "$BIN_PATH.new"
    mv -f "$BIN_PATH.new" "$BIN_PATH"
}

# @description Ortam dosyasını şablondan bir kez üretir ve izinlerini `root:root 0600` yapar.
# systemd `EnvironmentFile=`'ı root olarak okur; botun bu dosyayı okuması gerekmez.
# @noargs
install_env_file() {
    if [ ! -f "$ENV_FILE" ]; then
        if [ -f ./.env.example ]; then
            install -m 0600 -o root -g root ./.env.example "$ENV_FILE"
            echo "[*] $ENV_FILE oluşturuldu. Lütfen Discord Token ve API anahtarınızı girin!"
        fi
    else
        echo "[*] $ENV_FILE zaten mevcut, içerik korundu."
        chown root:root "$ENV_FILE"
        chmod 0600 "$ENV_FILE"
    fi
}

# @description Örnek ayar dosyasını, yoksa kalıcı veri dizinine kopyalar.
# @noargs
install_settings() {
    if [ -f ./settings.json ] && [ ! -f "$STATE_DIR/settings.json" ]; then
        install -m 0600 -o botgek -g botgek ./settings.json "$STATE_DIR/settings.json"
    fi
    chown -R botgek:botgek "$STATE_DIR"
}

# @description Birimi kurar, systemd'yi yeniler, servisi etkinleştirir ve çalışıyorsa yeniden başlatır.
# @noargs
install_service() {
    if [ ! -f ./botgek.service ]; then
        return 0
    fi
    echo "[*] Systemd servisi yapılandırılıyor..."
    install -m 0644 -o root -g root ./botgek.service "$UNIT_PATH"
    systemctl daemon-reload
    systemctl enable botgek.service
    if systemctl is-active --quiet botgek.service; then
        echo "[*] Çalışan servis yeni sürümle yeniden başlatılıyor..."
    fi
    systemctl try-restart botgek.service
}

# @description Kurulum sonrası adımları yazdırır.
# @noargs
# @stdout Sonraki adımlar.
print_next_steps() {
    cat <<EOF
=================================================================
 [OK] Botgek kurulumu tamamlandı!

 Sonraki Adımlar:
 1. Ayar dosyanızı düzenleyin (DISCORD_TOKEN zorunlu):
    sudo nano $ENV_FILE

 2. Botu başlatın (güncellemede çalışan servis zaten yeniden başlatıldı):
    sudo systemctl start botgek

 3. Bot durumunu ve logları takip edin:
    sudo systemctl status botgek
    sudo journalctl -u botgek -f

 4. WebUI Kontrol Paneli (varsayılan olarak KAPALI):
    - $ENV_FILE içinde ADMIN_TOKEN boşsa WebUI açılmaz; 'openssl rand -hex 32' ile üretip girin ve servisi yeniden başlatın.
    - Varsayılan WEB_BIND=127.0.0.1: panel yalnızca Pi'nin kendisinden erişilebilir (http://127.0.0.1:3000).
    - Başka bir bilgisayardan SSH tüneliyle açın: ssh -L 3000:127.0.0.1:3000 <kullanici>@<raspberry_pi_ip_adresi>
      ardından tarayıcıda http://127.0.0.1:3000
    - Portu doğrudan ağa açmayın; uzak erişim için HTTPS ters proxy + WEB_SECURE_COOKIE=true kullanın.
=================================================================
EOF
}

# @description Kurulum adımlarını sırayla çalıştırır.
# @noargs
main() {
    echo "=== Botgek Raspberry Pi Debian Kurulumu Başlatılıyor ==="
    require_root
    install_dependencies
    ensure_account
    prepare_directories
    install_binary
    install_env_file
    install_settings
    install_service
    print_next_steps
}

main "$@"

# =========================================================
# Aşama 1: Bağımlılıkları Önbelleğe Alma ve Derleme (Builder)
# =========================================================
FROM rust:1-bookworm AS builder

WORKDIR /usr/src/botgek

# Raspberry Pi derlemelerini hızlandırmak için bağımlılıkları önbellekle
COPY Cargo.toml ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

# Gerçek kaynak kodları kopyala ve nihai binary'yi derle
COPY src ./src
RUN touch src/main.rs && cargo build --release

# =========================================================
# Aşama 2: Minimal ve Güvenli Çalışma Zamanı (Runtime)
# =========================================================
FROM debian:bookworm-slim

# HTTPS istekleri için CA sertifikaları ve zaman dilimi paketi
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/*

# Güvenlik: Root olmayan bir kullanıcı oluştur
RUN useradd -m -u 1001 -U appuser
USER appuser
WORKDIR /app

# Derlenen ikili dosyayı kopyala
COPY --from=builder /usr/src/botgek/target/release/botgek /app/botgek

# Botu çalıştır
CMD ["/app/botgek"]

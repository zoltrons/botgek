# =========================================================
# Aşama 1: Bağımlılıkları Önbelleğe Alma ve Derleme (Builder)
# =========================================================
FROM rust:1-bookworm AS builder

WORKDIR /usr/src/botgek

# Bağımlılıkları tam önbelleğe almak için Cargo.toml ve Cargo.lock kopyalanır
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src target/release/deps/botgek* target/release/botgek*

# Gerçek kaynak kodları kopyala ve nihai binary'yi derle
COPY src ./src
RUN cargo build --release

# =========================================================
# Aşama 2: Minimal ve Güvenli Çalışma Zamanı (Runtime)
# =========================================================
FROM debian:bookworm-slim

# HTTPS istekleri için CA sertifikaları ve zaman dilimi paketi
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Derlenen ikili dosyayı kopyala
COPY --from=builder /usr/src/botgek/target/release/botgek /app/botgek

# Botu çalıştır
CMD ["/app/botgek"]

# syntax=docker/dockerfile:1.7
# =========================================================
# Aşama 1: Modern & Hızlı Derleme (Builder)
# =========================================================
FROM rust:1-bookworm AS builder

WORKDIR /usr/src/botgek

# LLD bağlayıcısını kurarak devasa Rust bağlama (linking) süresini 5-10 kat hızlandır
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends clang lld && rm -rf /var/lib/apt/lists/*
ENV RUSTFLAGS="-C link-arg=-fuse-ld=lld"

# 1. Adım: Bağımlılıkları önbelleğe al
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/usr/src/botgek/target,sharing=locked \
    cargo build --release

# 2. Adım: Gerçek kaynak kodları kopyala ve nihai binary'yi derle (önceki target cache'i yeniden kullanılır)
COPY src ./src
RUN touch src/main.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/usr/src/botgek/target,sharing=locked \
    cargo build --release && \
    cp target/release/botgek /usr/local/bin/botgek

# =========================================================
# Aşama 2: Minimal ve Güvenli Çalışma Zamanı (Runtime)
# =========================================================
FROM debian:bookworm-slim

# HTTPS istekleri için CA sertifikaları ve zaman dilimi paketi (önbellekli)
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && rm -rf /var/lib/apt/lists/*

RUN groupadd -g 1000 botgek && \
    useradd -u 1000 -g botgek -m -s /bin/sh botgek && \
    mkdir -p /app && chown -R botgek:botgek /app

USER botgek
WORKDIR /app

# Derlenen ikili dosyayı kopyala
COPY --from=builder --chown=botgek:botgek /usr/local/bin/botgek /app/botgek

# Botu çalıştır
CMD ["/app/botgek"]

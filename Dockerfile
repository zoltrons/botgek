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
# `build.rs` + `build-support/` derleme zamanı sürüm bilgisini üretir (git bağlamda yoktur):
# ikisi de kopyalanmazsa build script çalışmaz ve sürüm `+local` kalır.
COPY build.rs build.rs
COPY build-support ./build-support
RUN mkdir -p src && echo "fn main() {}" > src/main.rs
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/usr/local/cargo/git,sharing=locked \
    --mount=type=cache,target=/usr/src/botgek/target,sharing=locked \
    cargo build --release

# 2. Adım: Gerçek kaynak kodları kopyala ve nihai binary'yi derle (önceki target cache'i yeniden kullanılır)
COPY src ./src
COPY build.rs build.rs
COPY build-support ./build-support

# Sürüm künyesi ikiliye gömülür (Docker bağlamında `.git` yoktur). Kaynak derlemesinde
# etiketi elle verin, aksi hâlde `Cargo.toml` sürümü + `+local` raporlanır:
#   docker build --build-arg BOTGEK_VERSION=v0.2.0 --build-arg BOTGEK_SHA=$(git rev-parse --short HEAD) .
ARG BOTGEK_VERSION=""
ARG BOTGEK_SHA=""
ENV BOTGEK_BUILD_VERSION=$BOTGEK_VERSION \
    BOTGEK_GIT_SHA=$BOTGEK_SHA

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
# NOT: `ffmpeg` varsayılan olarak KURULMAZ — Debian'da tam paket ~75-95 MB bağımlılık
# çekip imajı şişirir. GIF'ler zaten FxEmbed'in servisinden hazır `.gif` olarak alınır
# (gif.fxtwitter.com); ffmpeg yalnızca o servis yanıt vermezse kullanılan yedektir.
# Yerel yedeği istiyorsan: docker build --build-arg WITH_FFMPEG=1 ...
ARG WITH_FFMPEG=0
RUN --mount=type=cache,target=/var/cache/apt,sharing=locked \
    --mount=type=cache,target=/var/lib/apt,sharing=locked \
    apt-get update && \
    PACKAGES="ca-certificates tzdata" && \
    if [ "$WITH_FFMPEG" = "1" ]; then PACKAGES="$PACKAGES ffmpeg"; fi && \
    apt-get install -y --no-install-recommends $PACKAGES && \
    rm -rf /var/lib/apt/lists/*

RUN groupadd -g 1000 botgek && \
    useradd -u 1000 -g botgek -m -s /bin/sh botgek && \
    mkdir -p /app /app/data && chown -R botgek:botgek /app

USER botgek
WORKDIR /app

# Derlenen ikili dosyayı kopyala
COPY --chmod=755 --from=builder --chown=botgek:botgek /usr/local/bin/botgek /app/botgek

# Botu çalıştır
CMD ["/app/botgek"]

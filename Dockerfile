# syntax=docker/dockerfile:1.7
#
# Multi-architecture image (linux/amd64, linux/arm64). The stages that compile run on the build
# machine's native platform and cross-compile, so arm64 images build fast; only the small runtime
# stage (apk add) runs under emulation, which needs QEMU (docker/setup-qemu-action in CI).

# --- Web interface -----------------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM node:24-alpine AS web
WORKDIR /web
RUN npm install --global pnpm@10.33.0
COPY web/package.json web/pnpm-lock.yaml ./
RUN --mount=type=cache,target=/root/.local/share/pnpm/store pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# --- Static binary (musl) ----------------------------------------------------------------------
FROM --platform=$BUILDPLATFORM rust:1-bookworm AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends python3-pip \
 && rm -rf /var/lib/apt/lists/* \
 && pip3 install --break-system-packages --no-cache-dir ziglang cargo-zigbuild \
 && rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY migrations ./migrations
COPY --from=web /web/dist ./web/dist
ARG TARGETARCH
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target,id=rustload-target-${TARGETARCH} \
    case "$TARGETARCH" in \
      amd64) triple=x86_64-unknown-linux-musl ;; \
      arm64) triple=aarch64-unknown-linux-musl ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && cargo zigbuild --release --locked --target "$triple" \
 && cp "target/$triple/release/rustload" /rustload

# --- 7-Zip ------------------------------------------------------------------------------------
# Official static build: distribution packages are compiled without the RAR codec.
FROM --platform=$BUILDPLATFORM alpine:3.22 AS sevenzip
ARG TARGETARCH
ARG SEVENZIP_VERSION=2602
ARG SEVENZIP_SHA256_AMD64=41aaba7b1235304ab5aa0624530c67ae829496cd29e875925271efdccc28c03e
ARG SEVENZIP_SHA256_ARM64=70ea6cc737ae1495ea2d7eb20ef3120fe579bd3f1a83a9d2362b62ec5bde2bba
RUN apk add --no-cache curl xz \
 && case "$TARGETARCH" in \
      amd64) arch=x64; sum="$SEVENZIP_SHA256_AMD64" ;; \
      arm64) arch=arm64; sum="$SEVENZIP_SHA256_ARM64" ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o /tmp/7z.tar.xz "https://7-zip.org/a/7z${SEVENZIP_VERSION}-linux-${arch}.tar.xz" \
 && echo "$sum  /tmp/7z.tar.xz" | sha256sum -c - \
 && tar -xJf /tmp/7z.tar.xz -C /tmp 7zzs \
 && install -m 755 /tmp/7zzs /7zz

# --- yt-dlp -----------------------------------------------------------------------------------
# Official standalone musl build (needs only musl and zlib). Media sites change often: the copy
# installed from Settings > Media into /config/bin takes precedence and is kept up to date.
FROM --platform=$BUILDPLATFORM alpine:3.22 AS ytdlp
ARG TARGETARCH
ARG YTDLP_VERSION=2026.08.19
ARG YTDLP_SHA256_AMD64=f3dec9cfeaf304cec98290fe41c6ad465d4b747d302473559643e7af24929722
ARG YTDLP_SHA256_ARM64=17b164c4d258be92bb1ad146cb7c336b783aedb380814aabbcb7d52937f77e57
RUN apk add --no-cache curl \
 && case "$TARGETARCH" in \
      amd64) asset=yt-dlp_musllinux; sum="$YTDLP_SHA256_AMD64" ;; \
      arm64) asset=yt-dlp_musllinux_aarch64; sum="$YTDLP_SHA256_ARM64" ;; \
      *) echo "unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o /tmp/yt-dlp "https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/${asset}" \
 && echo "$sum  /tmp/yt-dlp" | sha256sum -c - \
 && install -m 755 /tmp/yt-dlp /yt-dlp

# --- Runtime -----------------------------------------------------------------------------------
FROM alpine:3.22
# tini reaps 7-Zip, yt-dlp and hook processes and forwards signals. ffmpeg merges and converts
# media streams; QuickJS lets yt-dlp solve YouTube's JavaScript challenges.
RUN apk add --no-cache ca-certificates tzdata tini ffmpeg quickjs zlib
COPY --from=sevenzip /7zz /usr/local/bin/7zz
COPY --from=ytdlp /yt-dlp /usr/local/bin/yt-dlp
COPY --from=build /rustload /usr/local/bin/rustload

ENV RUSTLOAD_CONFIG_DIR=/config \
    RUSTLOAD_DOWNLOAD_DIR=/downloads \
    RUSTLOAD_PORT=8800 \
    RUSTLOAD_CNL_HOST=0.0.0.0 \
    RUSTLOAD_CNL_PORT=9666
VOLUME ["/config", "/downloads"]
EXPOSE 8800 9666

HEALTHCHECK --interval=30s --timeout=5s --start-period=15s \
  CMD wget -q -O /dev/null "http://127.0.0.1:${RUSTLOAD_PORT}/api/health" || exit 1

# Starts as root to apply PUID/PGID/UMASK, then drops privileges.
ENTRYPOINT ["/sbin/tini", "--", "/usr/local/bin/rustload"]

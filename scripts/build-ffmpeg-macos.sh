#!/bin/bash
# Compila FFmpeg e FFprobe estáticos para um alvo macOS, a partir dos códigos-fonte conferidos
# por scripts/fetch-binaries.py (vendor/ffmpeg-sources.json). Inclui x264, LAME, Opus,
# VideoToolbox e AudioToolbox. Receita adaptada do port para macOS do fork mecBOLADAO
# (https://github.com/leonardobluestriker/mecBOLADAO).
# Uso: build-ffmpeg-macos.sh <alvo> <pasta-com-os-fontes> <pasta-de-saida>
set -euo pipefail

target="$1"
sources="$2"
output="$3"
case "$target" in
  aarch64-apple-darwin) arch=arm64; ffmpeg_arch=aarch64 ;;
  x86_64-apple-darwin) arch=x86_64; ffmpeg_arch=x86_64 ;;
  *) echo "Alvo inválido: $target" >&2; exit 2 ;;
esac

jobs="${BOLADAO_BUILD_JOBS:-$(getconf _NPROCESSORS_ONLN)}"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
prefix="$work/install"
sdk="$(xcrun --sdk macosx --show-sdk-path)"
flags="-arch $arch -isysroot $sdk -mmacosx-version-min=12.0"

for component in pkgconf x264 lame opus ffmpeg; do
  mkdir "$work/$component"
  tar -xf "$sources/$component" -C "$work/$component" --strip-components 1
done

(
  cd "$work/pkgconf"
  ./configure --prefix="$work/pkgconf-native" --disable-shared
  make -j"$jobs" install
)
(
  cd "$work/x264"
  CC=clang CFLAGS="$flags" LDFLAGS="$flags" ./configure --host="$target" \
    --prefix="$prefix" --enable-static --disable-cli --disable-asm --disable-opencl
  make -j"$jobs" install
)
for component in lame opus; do
  (
    cd "$work/$component"
    options=()
    if [[ "$component" == lame ]]; then
      options+=(--disable-frontend --disable-decoder)
    fi
    CC=clang CFLAGS="$flags" LDFLAGS="$flags" ./configure --host="$target" \
      --prefix="$prefix" --enable-static --disable-shared ${options[@]+"${options[@]}"}
    make -j"$jobs" install
  )
done

(
  cd "$work/ffmpeg"
  export PKG_CONFIG_LIBDIR="$prefix/lib/pkgconfig"
  assembly=()
  if [[ "$arch" == x86_64 ]]; then
    assembly+=(--disable-x86asm)
  fi
  # --disable-autodetect também desliga o zlib, de que o PNG depende (capas, espectrograma, paleta
  # do GIF). zlib e bzip2 já vêm com o macOS. O iconv fica de fora: com --disable-autodetect, o
  # configure só o procura na libc, e no macOS ele está na libiconv, então forçá-lo quebra o link.
  ./configure --arch="$ffmpeg_arch" --target-os=darwin --enable-cross-compile \
    --cc=clang --extra-cflags="$flags -I$prefix/include" --extra-ldflags="$flags -L$prefix/lib" \
    --pkg-config="$work/pkgconf-native/bin/pkgconf" --pkg-config-flags=--static \
    --disable-autodetect --disable-doc --disable-debug --disable-ffplay \
    --enable-static --disable-shared --enable-gpl \
    --enable-libx264 --enable-libmp3lame --enable-libopus --enable-securetransport \
    --enable-videotoolbox --enable-audiotoolbox --enable-zlib --enable-bzlib \
    ${assembly[@]+"${assembly[@]}"}
  # O configure deixa de fora, sem erro, o que não encontra no SDK.
  grep -q '^#define CONFIG_PNG_ENCODER 1' config_components.h \
    || { echo "FFmpeg ficou sem PNG: o zlib do SDK não foi encontrado." >&2; exit 1; }
  grep -q '^#define CONFIG_SECURETRANSPORT 1' config.h \
    || { echo "FFmpeg ficou sem https: o SecureTransport do SDK não foi encontrado." >&2; exit 1; }
  make -j"$jobs" ffmpeg ffprobe
  /usr/bin/install -m 755 ffmpeg ffprobe "$output/"
  /usr/bin/install -m 644 COPYING.GPLv2 "$output/LICENSE"
)

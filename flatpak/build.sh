#!/usr/bin/env bash
# Construye Kandy como Flatpak, lo instala para el usuario y genera kandy.flatpak.
#
#   ./flatpak/build.sh            # compilar + instalar + empaquetar
#   ./flatpak/build.sh --no-bundle  # sin generar el archivo .flatpak
set -euo pipefail

cd "$(dirname "$0")/.."
APP_ID=io.github.fersot.Kandy
MANIFEST=flatpak/$APP_ID.yml

if command -v flatpak-builder >/dev/null 2>&1; then
  BUILDER=(flatpak-builder)
else
  BUILDER=(flatpak run org.flatpak.Builder)
fi

echo "==> Dependencias (runtime/SDK GNOME 51, Rust, Node)…"
flatpak install --user -y --noninteractive flathub \
  org.gnome.Platform//51 org.gnome.Sdk//51 \
  org.freedesktop.Sdk.Extension.rust-stable//26.08 \
  org.freedesktop.Sdk.Extension.node24//26.08 >/dev/null

echo "==> Compilando $APP_ID…"
"${BUILDER[@]}" --user --install --force-clean \
  --state-dir=.flatpak-builder \
  --repo=flatpak/repo \
  flatpak/build-dir "$MANIFEST"

if [[ "${1:-}" != "--no-bundle" ]]; then
  echo "==> Generando kandy.flatpak…"
  flatpak build-bundle flatpak/repo kandy.flatpak "$APP_ID" \
    --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo
  echo "Paquete listo: $(pwd)/kandy.flatpak"
fi

echo "Ejecuta:  flatpak run $APP_ID"

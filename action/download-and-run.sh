#!/usr/bin/env bash
# Composite-action шаг: скачать прекомпилированный бинарь chronograph, проверить
# его sha256 и запустить `chronograph report`.
#
# Пин по версии (CHRONOGRAPH_DEFAULT_VERSION == git-тег релиза Action) — чужой CI
# не ломается молча. Целостность — обязательная проверка sha256 до запуска.
# На старте поддержан только Linux x64; остальное — явная понятная ошибка.
set -euo pipefail

VERSION="${CHRONOGRAPH_VERSION_INPUT:-}"
if [ -z "${VERSION}" ]; then
  VERSION="${CHRONOGRAPH_DEFAULT_VERSION}"
fi

OS="${RUNNER_OS:-unknown}"
ARCH="${RUNNER_ARCH:-unknown}"
if [ "${OS}" != "Linux" ] || [ "${ARCH}" != "X64" ]; then
  echo "::error::chronograph-action пока поддерживает только Linux x64 runner'ы (получено ${OS}/${ARCH}). Поддержка macOS/Windows запланирована."
  exit 1
fi

TARGET="x86_64-unknown-linux-gnu"
ASSET="chronograph-${VERSION}-${TARGET}.tar.gz"
# Базовый URL релизов (переопределяется CHRONOGRAPH_BASE_URL для локального теста).
BASE_URL="${CHRONOGRAPH_BASE_URL:-https://github.com/${CHRONOGRAPH_REPO}/releases/download}"

workdir="$(mktemp -d)"
trap 'rm -rf "${workdir}"' EXIT

echo "Скачиваю ${ASSET} (${VERSION})..."
curl -fsSL -o "${workdir}/${ASSET}" "${BASE_URL}/${VERSION}/${ASSET}"
curl -fsSL -o "${workdir}/${ASSET}.sha256" "${BASE_URL}/${VERSION}/${ASSET}.sha256"

echo "Проверяю sha256..."
( cd "${workdir}" && sha256sum -c "${ASSET}.sha256" )

tar -xzf "${workdir}/${ASSET}" -C "${workdir}"
chmod +x "${workdir}/chronograph"

echo "Запускаю chronograph report..."
"${workdir}/chronograph" report "${CHRONOGRAPH_INPUT_PATH}" --out "${CHRONOGRAPH_OUTPUT}"
echo "Отчёт: ${CHRONOGRAPH_OUTPUT}"

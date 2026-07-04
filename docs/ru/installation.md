# Установка

[← Оглавление](README.md) · [English](../en/installation.md)

Chronograph — это Rust‑workspace. Отдельного канала готовых бинарников для локального использования пока нет, поэтому поддерживаемый путь — **сборка из исходников**. (GitHub Action использует прекомпилированный бинарь, приложенный к GitHub Release — см. [GitHub Action](github-action.md).)

## Требования

- **Rust** (stable) с `cargo` — установите через [rustup](https://rustup.rs/).
- **C/C++‑тулчейн** для компиляции встроенного DuckDB из исходников:
  - Linux: `gcc`/`clang` + `make`.
  - macOS: Xcode command‑line tools (`xcode-select --install`).
  - Windows: MSVC build tools (Visual Studio Build Tools).
- **git** в рантайме *не нужен* — Chronograph читает репозитории напрямую через `gix` (gitoxide). Git используется только тестовыми фикстурами.
- Для web UI: **Node.js** (18+) и npm.

## Сборка

```bash
git clone https://github.com/Nonaqe/Chronograph.git
cd Chronograph
cargo build --release
```

Бинарь появится по пути:

```
target/release/chronograph        # (chronograph.exe на Windows)
```

> **Первая сборка долгая.** Chronograph статически линкует встроенный DuckDB (собирается из исходников), грамматики tree‑sitter и gix. Первый `cargo build` может занять несколько минут; последующие сборки переиспользуют кэш. `cargo clean` выбрасывает этот кэш — не делайте его без необходимости.

Добавьте бинарь в `PATH` или вызывайте по полному пути (`./target/release/chronograph`).

## Проверка

```bash
chronograph --version
chronograph --help
```

Вы увидите список подкоманд: `analyze`, `hotspots`, `coupling`, `knowledge`, `age`, `report`, `export`.

## Первый запуск

Наведите Chronograph на любой git‑репозиторий:

```bash
chronograph hotspots /путь/к/репо
```

При первом запуске:

1. Открывается репозиторий через `gix`.
2. Обходится вся история — извлекаются коммиты, diff и переименования.
3. Пишется кэш `<repo>/.chronograph/cache.duckdb`.
4. Считается и печатается результат.

При последующих запусках обрабатываются только **новые коммиты** (запоминается последний `head_sha`), поэтому повторные команды быстрые.

## Кэш анализа

Chronograph хранит анализ в локальном файле DuckDB:

```
<repo>/.chronograph/cache.duckdb
```

- Его безопасно удалять — он пересоберётся при следующем запуске.
- Переместить его можно флагом `--db <FILE>` на любой команде.
- Добавьте `.chronograph/` в свой `.gitignore` (Chronograph сам его не коммитит).

Если после смены версии движка появились ошибки схемы — удалите кэш и перезапустите.

## О производительности

Для реальных репозиториев всегда используйте **release**‑бинарь. Debug‑сборки в 5–20 раз медленнее на CPU‑bound путях (парсинг сложности, blame). См. [Конфигурация → бюджет blame](configuration.md#бюджет-blame) для управления стоимостью на патологически больших файлах.

## Запуск web UI

```bash
cd web
npm install
npm run dev        # dev-сервер на http://localhost:5173
npm run build      # статика в web/dist/
```

UI нужен `chronograph.json`, созданный командой [`chronograph export`](cli.md#export). См. [руководство по Web UI](web-ui.md).

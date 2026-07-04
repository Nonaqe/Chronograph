# GitHub Action

[← Оглавление](README.md) · [English](../en/github-action.md)

GitHub Action — основной канал распространения Chronograph: добавьте его в workflow, и каждый прогон будет выдавать self‑contained HTML‑отчёт как artifact — без Rust‑тулчейна и без времени сборки.

Это **composite‑action**, который скачивает прекомпилированный бинарь из GitHub Release, **проверяет его SHA‑256**, запускает `chronograph report` и (опционально) загружает HTML.

## Минимальное использование

```yaml
name: chronograph
on: [push]
jobs:
  report:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0            # ОБЯЗАТЕЛЬНО: нужна полная история
      - uses: Nonaqe/Chronograph/action@v0.1.0
        with:
          path: .
          output: chronograph-report.html
```

> **`fetch-depth: 0` обязателен.** По умолчанию `actions/checkout` делает shallow‑клон с одним коммитом; Chronograph нужна полная история, чтобы посчитать хоть что‑то осмысленное.

Отчёт загружается как artifact `chronograph-report`. Скачать его можно во вкладке **Actions → нужный run → Artifacts**.

## Inputs

| Input | По умолчанию | Описание |
|---|---|---|
| `path` | `.` | Путь к git‑репозиторию для анализа. |
| `output` | `chronograph-report.html` | Куда записать HTML‑отчёт. |
| `version` | *(пин к релизу Action)* | Явно задать версию бинаря `chronograph`. |
| `upload-artifact` | `true` | Загружать ли отчёт как artifact. |

## Outputs

| Output | Описание |
|---|---|
| `report` | Путь к сгенерированному HTML‑отчёту. |

## Версионирование

Пинуйте Action по тегу (`@v0.1.0`) — он скачает **ровно ту же версию** бинаря. Бинарь меняется только когда вы меняете пин, поэтому движущийся upstream не сломает ваш CI молча. `latest` намеренно не используется.

## Платформы

На старте поддержаны только **Linux x64** runner'ы. На macOS/Windows runner'ах Action завершается понятной явной ошибкой (и ненулевым кодом), а не тихим сбоем. Другие платформы запланированы.

## Как это работает (и почему)

Сборка всего движка (gix + встроенный DuckDB + грамматики tree‑sitter) в CI занимала бы минуты на каждый прогон. Вместо этого:

1. Отдельный **release‑workflow** кросс‑компилирует бинарь Linux x64 по тегу `v*`, тарболит его, генерит `.sha256` и публикует оба в GitHub Release.
2. Action скачивает этот тарбол + контрольную сумму, **проверяет SHA‑256 перед запуском** (стандартная гигиена для action, которому доверяют чужие пайплайны), распаковывает и запускает `chronograph report`.

Поскольку скачивание — это анонимный `curl` релизного ассета, **upstream‑репозиторий обязан быть публичным** — приватные релизные ассеты не отдаются анонимным запросам. Это требование модели распространения, а не случайность.

## Публикация на GitHub Pages (опционально)

Превратите отчёт в просматриваемую страницу:

```yaml
      - uses: actions/upload-pages-artifact@v3
        with:
          path: chronograph-report.html
      - uses: actions/deploy-pages@v4
```

## О безопасности

Сторонние action в этих примерах запинены по major‑тегу для читаемости. В проде пинуйте их по полному commit SHA.

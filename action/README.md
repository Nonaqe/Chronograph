# Chronograph GitHub Action

Генерирует self-contained HTML-репорт эволюции кодовой базы (hotspots + change
coupling) и выкладывает его как artifact. Composite-action: скачивает
прекомпилированный бинарь из релиза, **проверяет его sha256**, запускает
`chronograph report`.

## Использование

```yaml
name: chronograph
on: [push]
jobs:
  report:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0        # нужна полная история git для анализа
      - uses: chronograph/chronograph/action@v0.1.0
        with:
          path: .
          output: chronograph-report.html
```

Отчёт автоматически загружается как artifact `chronograph-report`. Скачать его
можно во вкладке Actions → нужный run → Artifacts.

## Inputs

| Input | Default | Описание |
|---|---|---|
| `path` | `.` | Путь к git-репозиторию для анализа. |
| `output` | `chronograph-report.html` | Путь к выходному HTML. |
| `version` | *(пин к релизу Action)* | Явно задать версию бинаря chronograph. |
| `upload-artifact` | `true` | Загружать ли отчёт как artifact. |

## Версионирование

Пинуйте Action по тегу (`@v0.1.0`) — он скачает бинарь **ровно той же версии**.
Обновление бинаря происходит только при смене пина, чужой CI не ломается молча.

## Платформы

На старте поддержан только **Linux x64** runner. На macOS/Windows Action
завершится понятной ошибкой (поддержка запланирована), а не тихим сбоем.

## Публикация на GitHub Pages (опционально)

```yaml
      - uses: actions/upload-pages-artifact@v3
        with:
          path: chronograph-report.html
      - uses: actions/deploy-pages@v4
```

> Безопасность: сторонние Action (`actions/*`, `softprops/*`, `dtolnay/*`) в
> примерах запинены по major-тегу для читаемости; в проде рекомендуется пинить
> по полному commit SHA.

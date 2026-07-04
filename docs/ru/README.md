# Документация Chronograph

[← README проекта](../../README.ru.md) · [English](../en/README.md)

Добро пожаловать в документацию Chronograph. Chronograph — движок аналитики эволюции git‑репозиториев: он читает историю и выдаёт прозрачные детерминированные сигналы о том, где в кодовой базе риск.

## Содержание

1. **[Установка](installation.md)** — сборка из исходников, требования, бинарь, кэш.
2. **[Справочник CLI](cli.md)** — каждая команда (`analyze`, `hotspots`, `coupling`, `knowledge`, `age`, `report`, `export`) и каждый флаг с примерами вывода.
3. **[Метрики](metrics.md)** — точные определения и формулы churn, complexity, hotspots, change coupling, knowledge / bus factor и code age.
4. **[GitHub Action](github-action.md)** — запуск в CI, inputs, примеры workflow, публикация на GitHub Pages.
5. **[HTML‑отчёт](html-report.md)** — что содержит self‑contained `report.html` и как он собирается.
6. **[Web UI](web-ui.md)** — шесть интерактивных вкладок, запуск и загрузка данных, со скриншотами.
7. **[JSON‑экспорт](json-export.md)** — полная схема `chronograph.json` (версия 1).
8. **[Архитектура](architecture.md)** — крейты, границы зависимостей, поток данных, схема DuckDB и как гарантируется детерминизм.
9. **[Конфигурация](configuration.md)** — окна churn, glob‑исключения, порог механических коммитов, бюджет blame, support для coupling.
10. **[FAQ и решение проблем](faq.md)** — частые проблемы, приватность/анонимизация и анти‑цели проекта.

## Ментальная модель за 30 секунд

```
       история git
             │
       ┌─────▼─────┐   gix (gitoxide) обходит коммиты, diff, renames
       │  ingest   │   → в локальный кэш DuckDB (.chronograph/)
       └─────┬─────┘
             │
       ┌─────▼─────┐   churn · complexity (tree-sitter) · coupling
       │  metrics  │   knowledge (blame) · code age · hotspots
       └─────┬─────┘
             │
   ┌─────────┼──────────┐
   ▼         ▼          ▼
 CLI      report.html  chronograph.json ──► web UI
 таблицы  (Action)     (детерм. экспорт)
```

Всё инкрементально: сохраняется `head_sha` последнего прогона, поэтому повторный запуск обрабатывает только новые коммиты.

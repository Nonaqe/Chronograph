# Архитектура

[← Оглавление](README.md) · [English](../en/architecture.md)

Chronograph — это Cargo‑workspace со строгими границами крейтов. Границы — это и есть суть: они держат аналитический слой переиспользуемым и независимым от конкретного git‑бэкенда.

## Крейты

```
chronograph-core     модель данных, конфиг, оркестрация, общие трейты
chronograph-git      обёртка над gix: обход истории, diff, renames, чтение блобов
chronograph-lang     tree-sitter, complexity по языкам
chronograph-metrics  churn, coupling, knowledge, code age, hotspot — каждая метрика отдельным модулем
chronograph-store    DuckDB: схема, миграции, запись/чтение
chronograph-report   self-contained HTML + JSON-экспорт
chronograph-cli      бинарь `chronograph`
```

### Правила зависимостей

- **`chronograph-metrics` НЕ зависит от `chronograph-git`.** Он работает только с данными в сторе. Git‑слой *наполняет* стор; слой метрик его *читает*. Тяжёлые агрегации выполняются внутри DuckDB.
- **Ничто не зависит от `chronograph-cli`.** CLI — верхний потребитель; пайплайн переиспользуем как библиотека.
- **`chronograph-core` не тянет тяжёлых зависимостей** (нет gix / tree‑sitter / duckdb) — только модель данных и трейты.

Метрики получают нужный им контент (байты файлов для complexity, blame) через **трейты**, реализуемые `chronograph-git`, поэтому напрямую от gix не зависят:

- `CommitSource` — обход истории (реализует `GitSource`).
- `Store` — персистентность (реализует `DuckStore`).
- `BlobReader` — чтение git‑блоба по `blob_sha` (реализует `GitSource`).

## Поток данных

```
 ┌──────────────┐   run_analysis(source, store, config)
 │ chronograph- │   • gix обходит новые коммиты (инкрементально через head_sha)
 │ git (gix)    │   • извлекает sha/parents/author/time
 │              │   • tree-diff против первого родителя, детект переименований
 └──────┬───────┘   • построчные added/deleted, blob_sha
        │  пишет
        ▼
 ┌──────────────┐   кэш DuckDB в <repo>/.chronograph/cache.duckdb
 │ chronograph- │   authors, commits, file_changes (+ материализованные
 │ store (DuckDB)│  file_metrics, coupling, knowledge, file_age)
 └──────┬───────┘
        │  читает
        ▼
 ┌──────────────┐   compute_churn / compute_complexity / compute_coupling
 │ chronograph- │   compute_knowledge / compute_age / compute_hotspots
 │ metrics      │   materialize() пишет результаты обратно в стор
 └──────┬───────┘
        │
        ▼
   таблицы CLI · report.html · chronograph.json
```

Команды CLI вроде `hotspots`/`coupling` считают **на лету** ради свежести; `report`/`export` сначала **материализуют** аналитические таблицы. Обе стороны зовут одни и те же `compute_*`, поэтому расхождения нет — единый источник истины.

## Схема DuckDB

Стор персистит эти таблицы (стабильное подмножество — из ТЗ; `old_path`, `churn_365d`, `blob_sha` и `file_age` — согласованные расширения):

```sql
authors(author_id, canonical_name, canonical_email)
commits(sha, author_id, committed_at, files_changed, is_mechanical)
file_changes(sha, path, old_path, added, deleted, change_type, blob_sha)

-- материализованная аналитика
file_metrics(path, churn_total, churn_30d, churn_90d, churn_365d,
             complexity, complexity_per_loc, hotspot_rank, is_alive)
coupling(path_a, path_b, support, coupling_ratio, explained_by_imports)
knowledge(path, author_id, ownership_ratio)
module_bus_factor(module, bus_factor, top_owner_ratio)
file_age(path, lines, newest_age_days, median_age_days, p90_age_days, oldest_age_days)

analysis_meta(engine_version, config_hash, analyzed_at, head_sha)
```

DuckDB выбран ради **колоночной аналитики локально, без сервера** — self‑join co‑occurrence и оконные подсчёты churn выполняются в SQL.

## Инкрементальность

`analysis_meta.head_sha` хранит последний проанализированный head. На повторном прогоне старый head передаётся в rev‑walk как *скрытый* tip, поэтому обходятся только новые коммиты. Идемпотентные вставки (`INSERT OR IGNORE` по `commits.sha`) защищают от двойной обработки и правок истории.

## Детерминизм

Один репозиторий + один конфиг → **байт‑идентичный** вывод. Обеспечивается:

- **UTC везде.** Время — `i64` unix‑секунды в ядре; никакой session‑таймзоны.
- **Фиксированный порядок обхода.** Rev‑walk детерминирован; `author_id` назначается по первому появлению в этом порядке.
- **Детерминированная агрегация.** Ранг‑перцентили и сортировки тай‑брейкаются по пути; в выводе нет зависимости от порядка итерации `HashMap`.
- **Стабильная сериализация.** Ключи JSON сортированы, float с фиксированной точностью; числа в SVG `{:.2}`, цвета целочисленный `rgb()`.
- **Провенанс в каждом артефакте.** `engine_version`, `config_hash`, `head_sha` пишутся в каждый отчёт/экспорт. Wall‑clock `analyzed_at` держится вне ломающих байт‑идентичность позиций.

Тест воспроизводимости прогоняет пайплайн дважды и проверяет идентичность вывода; у HTML‑отчёта есть свой тест байт‑идентичности.

## Обработка ошибок

- Библиотечные крейты определяют свои типы ошибок через **`thiserror`** и избегают `unwrap()`/`expect()` вне доказуемо‑невозможных инвариантов.
- CLI использует **`anyhow`** с `.context(...)` для человекочитаемых сообщений.

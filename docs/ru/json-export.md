# Схема JSON-экспорта

[← Оглавление](README.md) · [English](../en/json-export.md)

`chronograph export` пишет один детерминированный `chronograph.json`. Это контракт между движком и [Web UI](web-ui.md), но файл самодостаточен для любого пайплайна. Здесь документируется **схема версии 1** (`meta.schema_version === 1`).

```bash
chronograph export /путь/к/репо --out chronograph.json
```

Документ **байт‑идентичен** между прогонами на одном коммите (сортированные ключи, фиксированный формат float).

## Верхнеуровневая структура

```jsonc
{
  "meta":        { ... },   // метаданные репозитория + прогона
  "files":       [ ... ],   // метрики по файлам (hotspots, churn, complexity)
  "coupling":    [ ... ],   // пары change-coupling
  "knowledge":   [ ... ],   // bus factor по файлам
  "file_age":    [ ... ],   // распределение возраста строк по файлам
  "blame_skips": [ ... ],   // файлы, пропущенные при blame, с причинами
  "events":      [ ... ]    // полный поток событий per-commit
}
```

## `meta`

```jsonc
{
  "schema_version": 1,
  "engine_version": "0.0.0",
  "config_hash": "f283ba95ccc573f7",   // хэш конфига анализа
  "head_sha": "7634900254ca…",          // коммит, который отражает экспорт
  "anchor_ts": 1622505600,              // unix-секунды UTC; от него отсчитаны age-дни
  "total_commits": 7,
  "total_authors": 1,
  "anonymized": true                    // true, если не использовался --show-names
}
```

## `files[]` — метрики файлов

Nullable‑поля равны `null`, когда неприменимы (например, нет сложности для неподдержанного языка).

```jsonc
{
  "path": "a.rs",
  "churn_total": 6,
  "churn_30d": 6,
  "churn_90d": 6,
  "churn_365d": 6,
  "complexity": 3.0,
  "complexity_per_loc": 3.0,
  "hotspot_rank": 1,        // 1 = самый горячий; null, если не ранжирован
  "is_alive": true          // false, если файл удалён
}
```

## `coupling[]` — change coupling

```jsonc
{ "a": "a.rs", "b": "c.rs", "support": 6, "ratio": 1.0 }
```

`a < b` канонически; `ratio = support / min(commits(a), commits(b))` ∈ (0, 1]. См. [Метрики → Change coupling](metrics.md#change-coupling).

## `knowledge[]` — bus factor

```jsonc
{ "path": "a.rs", "bus_factor": 1, "top_owner_ratio": 1.0, "top_owner": "Author #1" }
```

`top_owner` — это `Author #N`, если не передан `--show-names`.

## `file_age[]` — возраст строк

```jsonc
{
  "path": "a.rs",
  "lines": 1,
  "newest_age_days": 0,
  "median_age_days": 0,
  "p90_age_days": 0,
  "oldest_age_days": 0
}
```

Возраст в днях от `meta.anchor_ts`.

## `blame_skips[]` — пропущенные файлы

Файлы, слишком дорогие для blame (или где blame упал) — указываются, а не выбрасываются молча.

```jsonc
{ "path": "CHANGELOG.md", "reason": "over_budget", "cost": 37000000, "budget": 10000000 }
```

`reason` — это `over_budget` (с `cost`/`budget`) или `failed` (`cost`/`budget` = null).

## `events[]` — поток событий per‑commit

Полная история в детерминированном порядке. Питает вкладки Timeline и Repository growth.

```jsonc
{
  "sha": "2790fc39…",
  "ts": 1622505600,           // unix-секунды UTC
  "author": "Author #1",      // анонимно, если не --show-names
  "mechanical": false,        // был ли коммит механическим?
  "changes": [
    {
      "path": "a.rs",
      "type": "M",            // A добавление · M изменение · D удаление · R rename · C copy
      "old_path": null,       // прежний путь для R/C, иначе null
      "added": 1,
      "deleted": 1
    }
  ]
}
```

## Как это потреблять

TypeScript‑типы в `web/src/types.ts` точно зеркалят эту схему (`meta.schema_version === 1`). Если Rust‑экспорт меняется, эти типы меняются синхронно — относитесь к `schema_version` как к воротам совместимости.

> Вывод в `parquet` (`--format parquet`) — планируемый follow‑up; флаг зарезервирован, чтобы CLI не пришлось менять.

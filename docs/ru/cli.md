# Справочник CLI

[← Оглавление](README.md) · [English](../en/cli.md)

Бинарь `chronograph` предоставляет семь подкоманд. Каждая аналитическая команда принимает путь к репозиторию (по умолчанию — текущая директория) и разделяет общий набор флагов.

```
chronograph <COMMAND> [PATH] [OPTIONS]

Команды:
  analyze    Полный/инкрементальный анализ истории; строит кэш .chronograph/cache.duckdb
  hotspots   Топ hotspots (churn × complexity) в терминал
  coupling   Топ change-coupling пар (файлы, меняющиеся вместе) в терминал
  knowledge  Риск концентрации знаний (bus factor) по файлам в терминал
  age        Распределение возраста строк (code age / stability) по файлам в терминал
  report     Self-contained HTML-отчёт (Overview + Hotspots + Coupling + Knowledge)
  export     Детерминированный JSON-экспорт метрик + потока событий (для Web UI/пайплайнов)
```

Каждая команда, читающая репозиторий, сначала выполняет **инкрементальный analyze**, поэтому `hotspots`/`coupling`/`report`/`export` можно вызывать напрямую — кэш автоматически доводится до `HEAD`.

## Общие флаги

| Флаг | Где | Значение |
|---|---|---|
| `[PATH]` | все | Путь к git‑репозиторию. По умолчанию `.` |
| `--db <FILE>` | все | Расположение кэша. По умолчанию `<repo>/.chronograph/cache.duckdb` |
| `--exclude <GLOB>` | все | Glob исключаемых путей (vendored/generated). Можно повторять. |
| `--top <N>` | hotspots, coupling, knowledge, age | Сколько строк печатать. По умолчанию `20` |
| `--show-names` | knowledge, export | Показать реальные имена авторов вместо `Author #N`. По умолчанию **выкл** |
| `--blame-budget <N>` | knowledge, age, report, export | Бюджет blame на файл (ревизии × добавленные строки). `0` = безлимит. По умолчанию `10000000` |

---

## `analyze`

Строит или обновляет кэш. Напрямую нужен редко (другие команды делают это сами), но полезен для прогрева кэша или форсирования полного пересчёта.

```
chronograph analyze [PATH] [OPTIONS]

Опции:
      --db <FILE>                 Файл кэша (по умолчанию <repo>/.chronograph/cache.duckdb)
      --exclude <GLOB>            Исключить пути (можно повторять)
      --no-incremental            Форсировать полный пересчёт вместо инкрементального
      --mechanical-max-files <N>  Помечать коммиты, тронувшие > N файлов, как «механические»
```

**Пример:**

```bash
chronograph analyze .
# Обработано новых коммитов: 925. HEAD: 7d3a…
# Кэш: ./.chronograph/cache.duckdb
```

При повторном запуске без новых коммитов:

```
Кэш актуален (HEAD 7d3a…); новых коммитов нет.
```

См. [Конфигурацию](configuration.md) про `--no-incremental` и `--mechanical-max-files`.

---

## `hotspots`

Ранжирует файлы по `churn × complexity`. Зона наибольшего риска сопровождения: часто меняется **и** структурно сложен.

```
chronograph hotspots [PATH] [--top N] [--db FILE] [--exclude GLOB]
```

**Пример вывода** (колонки: ранг, путь, churn, cyclomatic complexity, перцентиль churn, перцентиль сложности, score):

```
  #  path                                          churn    cx  churn%    cx%  score
  1  build.rs                                        106    24    0.98   0.99  0.970
  2  src/error.rs                                    199    10    1.00   0.82  0.820
  3  src/context.rs                                   61     9    0.79   0.79  0.624
  4  src/fmt.rs                                       34    15    0.55   0.93  0.512
  7  src/lib.rs                                      301     2    1.00   0.20  0.200
```

Обратите внимание на `src/lib.rs`: огромный churn, но тривиальная сложность → низкий ранг. В этом и суть — один churn ещё не риск.

> Ранжируются только **живые файлы с cyclomatic complexity** (Rust/Python/Go/JS/TS). Файлы, попадающие на indentation‑fallback (доки, конфиги, неподдержанные языки), из hotspot‑рейтинга **исключены**. См. [Метрики → Hotspots](metrics.md#hotspots).

---

## `coupling`

Находит файлы, которые **меняются вместе**. Вскрывает скрытые архитектурные зависимости, невидимые по импортам.

```
chronograph coupling [PATH] [--top N] [--min-support N] [--db FILE] [--exclude GLOB]
```

- `--min-support <N>` — минимальное число совместных коммитов, чтобы пара попала в рейтинг. По умолчанию **5**.

**Пример вывода** (колонки: support, ratio, файл A, файл B):

```
 supp  ratio  file_a                             file_b
   23   0.92  src/error.rs                       src/wrapper.rs
   31   0.79  src/backtrace.rs                   src/error.rs
   20   0.80  src/context.rs                     src/wrapper.rs
   12   0.75  build.rs                           build/probe.rs
```

`error.rs ↔ wrapper.rs` с ratio 0.92 — тот самый инсайт «скрытого долга»: они архитектурно сцеплены, хотя ни один не импортирует другой. См. [Метрики → Change coupling](metrics.md#change-coupling).

---

## `knowledge`

Считает **концентрацию знаний** (bus factor) по файлам из `git blame`. Сортировка **по риску** (наверху — минимальный bus factor и максимальная доля топ‑владельца).

```
chronograph knowledge [PATH] [--top N] [--show-names] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

**Авторы анонимизированы по умолчанию** (`Author #N`) — это продуктовое требование, а не вкусовщина (метрика про риск, не про вину). Флаг `--show-names` показывает реальные имена.

**Пример вывода:**

```
файлов: 118; bus_factor = 1 (риск концентрации): 63; пропущено blame: 0
 bf  top%  top owner                 file
  1  100%  Author #1                 src/format.rs
  1   96%  Author #1                 src/backtrace.rs
  1   88%  Author #2                 build.rs
  2   61%  Author #1                 src/error.rs
```

**Bus factor = 1** означает, что один автор владеет более чем половиной живых строк файла — если он уйдёт, знание уйдёт с ним.

Файлы, слишком дорогие для blame (см. [`--blame-budget`](configuration.md#бюджет-blame)), пропускаются и явно указываются, а не выбрасываются молча.

---

## `age`

Показывает **распределение возраста живых строк** по файлам из `git blame`. Возраст в днях от `anchor = max(committed_at)` (детерминированно, не wall‑clock).

```
chronograph age [PATH] [--top N] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

**Пример вывода** (сортировка по минимальной медиане — самый переписываемый код наверху):

```
файлов: 118; медиана median-возраста: 612 дн.; пропущено blame: 0
 newest  median     p90  oldest  file
      0       4      31      95  src/error.rs
      2      18      66     140  src/context.rs
     12     420     900    1400  src/lib.rs
```

- **Малая медиана** → зона постоянного переписывания.
- **Большая медиана** → стабильный старый код.

---

## `report`

Генерирует один **self‑contained HTML‑отчёт** (ноль внешних запросов): Overview + treemap Hotspots + Coupling + Knowledge.

```
chronograph report [PATH] [--out FILE] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

- `--out <FILE>` — путь вывода. По умолчанию `report.html`.

```bash
chronograph report . --out report.html
# Отчёт записан: report.html
```

Два прогона на одном коммите дают **байт‑идентичный** файл. См. [HTML‑отчёт](html-report.md).

---

## `export`

Создаёт **детерминированный `chronograph.json`**, потребляемый [Web UI](web-ui.md) — метрики плюс полный поток событий per‑commit.

```
chronograph export [PATH] [--out FILE] [--format json] [--show-names] [--blame-budget N] [--db FILE] [--exclude GLOB]
```

- `--out <FILE>` — путь вывода. По умолчанию `chronograph.json`.
- `--format <json>` — пока только `json` (`parquet` — планируемый follow‑up).
- `--show-names` — включить реальные имена авторов (по умолчанию анонимно).

```bash
chronograph export . --out chronograph.json
# Экспорт записан: chronograph.json
```

Документ байт‑идентичен между прогонами на одном коммите. См. полную [схему JSON‑экспорта](json-export.md).

---

## Коды возврата и ошибки

Chronograph использует человекочитаемые контекстные сообщения об ошибках (через `anyhow`). Ненулевой код возврата означает сбой прогона (например, путь — не git‑репозиторий или кэш не удалось записать). Сбои blame на отдельных файлах **не** фатальны — они подсчитываются и указываются.

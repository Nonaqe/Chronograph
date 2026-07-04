<div align="center">

# Chronograph

**Аналитика эволюции git‑репозиториев.**

Превращает историю репозитория в три практических сигнала — **hotspots**, **change coupling** и **карту знаний / bus factor** — плюс возраст кода и анимацию роста репозитория.

[![CI](https://github.com/Nonaqe/Chronograph/actions/workflows/ci.yml/badge.svg)](https://github.com/Nonaqe/Chronograph/actions/workflows/ci.yml)
[![Rust](https://img.shields.io/badge/Rust-workspace-CE422B?logo=rust&logoColor=white)](Cargo.toml)
[![License](https://img.shields.io/badge/license-PolyForm_Noncommercial_1.0.0-blue)](LICENSE)

[English](README.md) · **Русский**

</div>

---

Chronograph — source‑available движок, который читает историю git и **детерминированно** вычисляет, *где* в кодовой базе риск и *как* он там появился — при этом никогда не оценивая отдельных разработчиков. Ядро написано на Rust (библиотека + CLI); основной канал распространения — **GitHub Action**, генерирующий один self‑contained HTML‑отчёт; опциональный **web‑интерфейс** превращает те же данные в интерактивные визуализации.

<div align="center">
<img src="docs/assets/coupling.png" alt="Force-граф change coupling" width="90%">
<br><em>Change coupling — файлы, меняющиеся вместе, в виде интерактивного force‑графа.</em>
</div>

## Зачем

Большинство инструментов качества кода говорят, как файл выглядит *прямо сейчас*. Chronograph говорит, что об этом файле знает его **история**:

- Файл с высоким **churn** и высокой **сложностью** — это *hotspot*: код, который постоянно трогают и меньше всего понимают.
- Файлы, которые постоянно меняются вместе (**change coupling**), вскрывают скрытые архитектурные зависимости, которых не видно по импортам.
- Файл, который осмысленно правил лишь один человек (**bus factor 1**), — риск концентрации знаний.

Эти сигналы коррелируют с реальными дефектами и реальной болью сопровождения, и ни один из них не требует ранжировать людей.

## Сигналы кратко

| Сигнал | На какой вопрос отвечает | Визуализация |
|---|---|---|
| **Hotspots** | Какие файлы одновременно часто меняются и сложны? | Zoomable treemap |
| **Change coupling** | Какие файлы меняются вместе, но лежат врозь? | Force‑граф |
| **Knowledge / bus factor** | Где знания концентрированы опасно? | Treemap риска + таблица |
| **Code age** | Какой код переписывается, а какой стабилен? | Гистограмма + карта возраста |
| **Timeline** | Как выглядело дерево в любой момент прошлого? | Ползунок + снимок |
| **Repository growth** | Как проект рос во времени? | Анимация «корней» |

<table>
<tr>
<td width="50%"><img src="docs/assets/hotspots.png" alt="Treemap hotspots"><br><em>Hotspots — площадь = сложность, цвет = churn.</em></td>
<td width="50%"><img src="docs/assets/knowledge.png" alt="Карта знаний"><br><em>Knowledge — цвет = bus factor, таблица ранжирует риск.</em></td>
</tr>
<tr>
<td width="50%"><img src="docs/assets/age.png" alt="Возраст кода"><br><em>Code age — распределение возраста строк по файлам.</em></td>
<td width="50%"><img src="docs/assets/growth.png" alt="Рост репозитория"><br><em>Repository growth — дерево растёт во времени.</em></td>
</tr>
</table>

## Быстрый старт

### 1. Собрать CLI

```bash
git clone https://github.com/Nonaqe/Chronograph.git
cd Chronograph
cargo build --release
# бинарь в target/release/chronograph
```

> Первая сборка компилирует встроенный DuckDB из исходников (несколько минут на холодном кэше). Дальнейшие сборки — инкрементальные.

### 2. Проанализировать репозиторий

```bash
# топ hotspots
chronograph hotspots /путь/к/репо

# файлы, меняющиеся вместе
chronograph coupling /путь/к/репо --min-support 5

# концентрация знаний (авторы анонимны по умолчанию)
chronograph knowledge /путь/к/репо

# self-contained HTML-отчёт
chronograph report /путь/к/репо --out report.html
```

### 3. (Опционально) Открыть в браузере

```bash
# детерминированный JSON-экспорт
chronograph export /путь/к/репо --out chronograph.json

# запустить web UI и перетащить в него chronograph.json
cd web && npm install && npm run dev   # http://localhost:5173
```

## В CI (GitHub Action)

Добавьте workflow, генерирующий HTML‑отчёт на каждый push и загружающий его как artifact:

```yaml
name: chronograph
on: [push]
jobs:
  report:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0            # нужна полная история git
      - uses: Nonaqe/Chronograph/action@v0.1.0
        with:
          path: .
          output: chronograph-report.html
```

Action скачивает прекомпилированный бинарь, проверяет его SHA‑256, запускает `chronograph report` и загружает результат. См. **[документацию по Action](docs/ru/github-action.md)**.

## Документация

| Раздел | Что внутри |
|---|---|
| [Установка](docs/ru/installation.md) | Сборка из исходников, требования, запуск бинаря |
| [Справочник CLI](docs/ru/cli.md) | Каждая команда и флаг с примерами вывода |
| [Метрики](docs/ru/metrics.md) | Точные определения и формулы всех сигналов |
| [GitHub Action](docs/ru/github-action.md) | Inputs, примеры workflow, GitHub Pages |
| [HTML‑отчёт](docs/ru/html-report.md) | Что содержит self‑contained отчёт |
| [Web UI](docs/ru/web-ui.md) | Шесть интерактивных вкладок со скриншотами |
| [JSON‑экспорт](docs/ru/json-export.md) | Схема `chronograph.json` (v1) |
| [Архитектура](docs/ru/architecture.md) | Крейты, поток данных, схема DuckDB, детерминизм |
| [Конфигурация](docs/ru/configuration.md) | Пороги, исключения, бюджет blame |
| [FAQ и решение проблем](docs/ru/faq.md) | Частые вопросы, приватность, анти‑цели |

## Принципы дизайна

1. **Сигнал прежде красоты.** Каждая возможность полезна из CLI без всякой графики. Анимация — *последняя* фича, а не первая.
2. **Метрики про файлы и модули, не про людей.** Карта знаний подаётся только как *риск концентрации* (bus factor), с анонимизацией по умолчанию.
3. **Никаких магических «health score 0–100».** Только прозрачные компоненты с задокументированными определениями; агрегаты всегда раскрываемы до составляющих.
4. **Детерминизм обязателен.** Один репозиторий + один конфиг → байт‑в‑байт одинаковый вывод. Все таймстемпы — UTC. Версия движка и хэш конфига пишутся в каждый отчёт.
5. **Производительность — требование, а не «оптимизация потом».** Инкрементальный анализ встроен: на повторных прогонах обрабатываются только новые коммиты.

## Чем Chronograph **не** является

- ❌ Индивидуальная оценка продуктивности разработчиков
- ❌ Менеджерские DORA‑метрики
- ❌ Линтер реального времени
- ❌ «Поддержка всех языков сразу» — на старте: JS/TS, Python, Go, Rust
- ❌ ML‑предсказание дефектов

## Стек

**gix** (gitoxide) для доступа к git · **tree‑sitter** для сложности по AST · **DuckDB** (bundled) для колоночной аналитики · **clap** для CLI · **rayon** для параллелизма · **React + D3** для web‑интерфейса.

## Лицензия

Лицензировано под **[PolyForm Noncommercial License 1.0.0](LICENSE)**.

Простыми словами: можно **бесплатно использовать, копировать, изменять, распространять и форкать Chronograph для любых некоммерческих целей** — личные проекты, учёба, исследования, хобби, образование, некоммерческие и государственные организации — при условии сохранения строки `Required Notice` (указание автора) в распространяемых копиях. **Коммерческое использование / монетизация требует отдельной лицензии** от автора. ПО поставляется «как есть», без гарантий.

> Это *source‑available* лицензия, а не OSI‑одобренная open‑source (она ограничивает коммерческое использование). См. [полный текст](LICENSE).

---

<div align="center">
<sub>Сделано на Rust · <a href="docs/ru/architecture.md">архитектура</a> · <a href="README.md">English version</a></sub>
</div>

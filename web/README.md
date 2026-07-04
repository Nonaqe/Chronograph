# Chronograph Web UI

Опциональный интерактивный клиент (ТЗ §6.4). Читает `chronograph.json` —
детерминированный экспорт движка:

```
chronograph export [PATH] --out chronograph.json
```

## Запуск

```
npm install
npm run dev        # dev-сервер (vite), http://localhost:5173
npm run build      # статика в dist/ (tsc --noEmit + vite build)
```

Данные загружаются drag-drop'ом файла `chronograph.json` (или кнопкой), а при
отдаче по http — параметром `?src=<url>` (например `?src=/data/ripgrep.json`,
положив файл в `public/data/` — каталог в .gitignore).

## Принципы

- Стек: React + TypeScript + Vite, визуализации — кастомные на D3 (§5 ТЗ).
  Все зависимости через npm, бандлятся в статику — ноль CDN.
- Правило байт-идентичности здесь НЕ действует (живой рендер, физика раскладки);
  детерминированный артефакт — сам `chronograph.json` (см. CONTEXT.md, Этап 5c).
- Авторы в данных анонимизированы движком по умолчанию (принцип 2.4);
  UI показывает то, что в файле.
- Пороги в контролах (ratio/support/топ-N) — презентационные фильтры
  отображения, не пороги метрик: данные приходят уже посчитанными.

## Структура

- `src/types.ts` — типы схемы экспорта (зеркало `chronograph-report/src/export.rs`,
  `meta.schema_version === 1`).
- `src/views/CouplingGraph.tsx` — force-graph change coupling (сделан).
- Hotspots treemap, Knowledge, Code age, Timeline scrubber, Gource-анимация —
  следующие шаги Этапа 5c (вкладки-заглушки).

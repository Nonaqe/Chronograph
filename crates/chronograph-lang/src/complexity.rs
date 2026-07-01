//! Подсчёт сложности файла.
//!
//! **Что считаем:** для поддержанных языков — cyclomatic complexity (число точек
//! ветвления + 1), обходом AST. Для прочих — indentation-based fallback.
//!
//! **Зачем:** сложный код дороже менять и легче сломать; в связке с churn даёт
//! hotspot. Cyclomatic выбран как прозрачная, объяснимая метрика с чётким
//! определением (а не «индекс из воздуха», принцип 2.6 ТЗ).
//!
//! **Определение cyclomatic здесь:** `1 + (число узлов-ветвлений)`, где
//! ветвления — это управляющие конструкции (if/elif, циклы, ветви match/switch/
//! select, except/catch, тернарный оператор). Логические операторы `&&`/`||` в
//! v1 НЕ учитываются (упрощение: их узлы в грамматиках не выделены отдельно и
//! требуют разбора оператора; задокументировано, при необходимости добавим).

use tree_sitter::{Language, Node, Parser};

/// Языки с поддержкой AST-сложности.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupportedLanguage {
    Rust,
    Python,
    Go,
    JavaScript,
    TypeScript,
    Tsx,
}

/// Способ, которым посчитана сложность (для прозрачности — разные шкалы).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityMethod {
    /// Cyclomatic по AST (поддержанные языки).
    Cyclomatic,
    /// Грубый fallback по глубине отступов (неподдержанные языки / ошибка парсинга).
    Indentation,
}

/// Сложность одного файла.
#[derive(Debug, Clone, PartialEq)]
pub struct FileComplexity {
    /// Язык, если распознан и поддержан (иначе fallback).
    pub language: Option<SupportedLanguage>,
    /// Метод подсчёта.
    pub method: ComplexityMethod,
    /// Значение сложности (cyclomatic-число или indentation-метрика).
    pub value: f64,
    /// Строк кода (непустых) — для нормализации.
    pub loc: u32,
    /// Сложность на строку (`value / loc`, 0 при пустом файле).
    pub per_loc: f64,
}

impl SupportedLanguage {
    /// Распознать язык по расширению пути. `None` → fallback.
    pub fn detect_by_path(path: &str) -> Option<Self> {
        let ext = path.rsplit('.').next().filter(|e| !e.contains('/'))?;
        // Если в пути нет точки, rsplit вернёт весь путь — отсекаем по '/'.
        if path
            .rsplit('/')
            .next()
            .map(|f| !f.contains('.'))
            .unwrap_or(true)
        {
            return None;
        }
        Some(match ext {
            "rs" => SupportedLanguage::Rust,
            "py" | "pyi" => SupportedLanguage::Python,
            "go" => SupportedLanguage::Go,
            "js" | "jsx" | "mjs" | "cjs" => SupportedLanguage::JavaScript,
            "ts" | "mts" | "cts" => SupportedLanguage::TypeScript,
            "tsx" => SupportedLanguage::Tsx,
            _ => return None,
        })
    }

    /// `tree_sitter::Language` для этого языка.
    fn ts_language(self) -> Language {
        match self {
            SupportedLanguage::Rust => tree_sitter_rust::LANGUAGE.into(),
            SupportedLanguage::Python => tree_sitter_python::LANGUAGE.into(),
            SupportedLanguage::Go => tree_sitter_go::LANGUAGE.into(),
            SupportedLanguage::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            SupportedLanguage::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            SupportedLanguage::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    /// Узлы-ветвления, увеличивающие cyclomatic (имена node-kind грамматики).
    fn decision_kinds(self) -> &'static [&'static str] {
        match self {
            SupportedLanguage::Rust => &[
                "if_expression",
                "match_arm",
                "while_expression",
                "for_expression",
                "loop_expression",
            ],
            SupportedLanguage::Python => &[
                "if_statement",
                "elif_clause",
                "for_statement",
                "while_statement",
                "except_clause",
                "case_clause",
                "conditional_expression",
            ],
            SupportedLanguage::Go => &[
                "if_statement",
                "for_statement",
                "expression_case",
                "type_case",
                "communication_case",
            ],
            // TS/TSX используют грамматику на базе JS — те же node-kind ветвлений.
            SupportedLanguage::JavaScript
            | SupportedLanguage::TypeScript
            | SupportedLanguage::Tsx => &[
                "if_statement",
                "for_statement",
                "for_in_statement",
                "while_statement",
                "do_statement",
                "switch_case",
                "catch_clause",
                "ternary_expression",
            ],
        }
    }
}

/// Посчитать сложность файла по пути и содержимому.
///
/// Поддержанный язык + успешный парсинг → cyclomatic; иначе → indentation-fallback.
pub fn file_complexity(path: &str, source: &[u8]) -> FileComplexity {
    let loc = count_loc(source);
    let language = SupportedLanguage::detect_by_path(path);

    let (value, method) = match language {
        Some(lang) => match cyclomatic(lang, source) {
            Some(c) => (c as f64, ComplexityMethod::Cyclomatic),
            None => (
                indentation_complexity(source) as f64,
                ComplexityMethod::Indentation,
            ),
        },
        None => (
            indentation_complexity(source) as f64,
            ComplexityMethod::Indentation,
        ),
    };

    let per_loc = if loc > 0 { value / loc as f64 } else { 0.0 };
    // Язык фиксируем только если реально считали по AST.
    let language = if method == ComplexityMethod::Cyclomatic {
        language
    } else {
        None
    };

    FileComplexity {
        language,
        method,
        value,
        loc,
        per_loc,
    }
}

/// Cyclomatic complexity = 1 + число узлов-ветвлений. `None` при сбое парсинга.
pub fn cyclomatic(lang: SupportedLanguage, source: &[u8]) -> Option<u32> {
    let mut parser = Parser::new();
    parser.set_language(&lang.ts_language()).ok()?;
    let tree = parser.parse(source, None)?;
    let kinds = lang.decision_kinds();
    let decisions = count_decision_nodes(tree.root_node(), kinds);
    Some(1 + decisions)
}

/// DFS по дереву: считает узлы, чей `kind()` входит в `kinds`.
fn count_decision_nodes(root: Node, kinds: &[&str]) -> u32 {
    let mut count = 0u32;
    let mut cursor = root.walk();
    'walk: loop {
        if kinds.contains(&cursor.node().kind()) {
            count += 1;
        }
        if cursor.goto_first_child() {
            continue 'walk;
        }
        loop {
            if cursor.goto_next_sibling() {
                continue 'walk;
            }
            if !cursor.goto_parent() {
                break 'walk;
            }
        }
    }
    count
}

/// Indentation-based fallback: сумма глубин вложенности по непустым строкам.
///
/// Глубина определяется относительным стеком отступов (без привязки к ширине
/// отступа — устойчиво к табам/пробелам). Грубо, но даёт ненулевой сигнал
/// «насколько вложен код» там, где AST недоступен (ТЗ 3.2).
pub fn indentation_complexity(source: &[u8]) -> u32 {
    let text = String::from_utf8_lossy(source);
    let mut stack: Vec<usize> = Vec::new();
    let mut total: u32 = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let indent = line.len() - line.trim_start().len();
        while let Some(&top) = stack.last() {
            if top >= indent {
                stack.pop();
            } else {
                break;
            }
        }
        total = total.saturating_add(stack.len() as u32);
        stack.push(indent);
    }
    total
}

/// Число непустых строк (LOC для нормализации).
fn count_loc(source: &[u8]) -> u32 {
    String::from_utf8_lossy(source)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .count() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_languages_by_extension() {
        use SupportedLanguage::*;
        assert_eq!(SupportedLanguage::detect_by_path("src/lib.rs"), Some(Rust));
        assert_eq!(SupportedLanguage::detect_by_path("a/b.py"), Some(Python));
        assert_eq!(SupportedLanguage::detect_by_path("main.go"), Some(Go));
        assert_eq!(
            SupportedLanguage::detect_by_path("app.js"),
            Some(JavaScript)
        );
        assert_eq!(
            SupportedLanguage::detect_by_path("app.ts"),
            Some(TypeScript)
        );
        assert_eq!(SupportedLanguage::detect_by_path("ui.tsx"), Some(Tsx));
        assert_eq!(SupportedLanguage::detect_by_path("data.json"), None);
        assert_eq!(SupportedLanguage::detect_by_path("Makefile"), None);
        assert_eq!(SupportedLanguage::detect_by_path("dir.with.dot/file"), None);
    }

    // --- cyclomatic по фикстурам с заранее известной сложностью ---

    #[test]
    fn rust_cyclomatic_counts_branches() {
        // if(1) + for(1) + if(1) + 2 match_arm(2) = 5 ветвлений → 1+5 = 6.
        let src = r#"
fn f(x: i32) -> i32 {
    if x > 0 {
        for i in 0..x {
            if i % 2 == 0 { return i; }
        }
    }
    match x {
        0 => 1,
        _ => 2,
    }
}
"#;
        assert_eq!(cyclomatic(SupportedLanguage::Rust, src.as_bytes()), Some(6));
    }

    #[test]
    fn python_cyclomatic_counts_branches() {
        // if + for + if + elif + while = 5 → 6.
        let src = r#"
def f(x):
    if x > 0:
        for i in range(x):
            if i % 2 == 0:
                return i
            elif i == 3:
                return -1
    while x > 0:
        x -= 1
    return x
"#;
        assert_eq!(
            cyclomatic(SupportedLanguage::Python, src.as_bytes()),
            Some(6)
        );
    }

    #[test]
    fn go_cyclomatic_counts_branches() {
        // if + for + if + 2 case (default не считается) = 5 → 6.
        let src = r#"
package main
func f(x int) int {
    if x > 0 {
        for i := 0; i < x; i++ {
            if i == 2 {
                return i
            }
        }
    }
    switch x {
    case 0:
        return 1
    case 1:
        return 2
    default:
        return 3
    }
}
"#;
        assert_eq!(cyclomatic(SupportedLanguage::Go, src.as_bytes()), Some(6));
    }

    #[test]
    fn javascript_cyclomatic_counts_branches() {
        // if + for + if + 2 case + ternary = 6 → 7 (default не считается).
        let src = r#"
function f(x) {
  if (x > 0) {
    for (let i = 0; i < x; i++) {
      if (i % 2 === 0) return i;
    }
  }
  switch (x) {
    case 0: return 1;
    case 1: return 2;
    default: return 3;
  }
  return x > 5 ? 1 : 0;
}
"#;
        assert_eq!(
            cyclomatic(SupportedLanguage::JavaScript, src.as_bytes()),
            Some(7)
        );
    }

    #[test]
    fn typescript_cyclomatic_counts_branches() {
        // if + if + while = 3 → 4.
        let src = r#"
function f(x: number): number {
  if (x > 0) {
    if (x > 10) {
      return 10;
    }
  }
  while (x > 0) {
    x -= 1;
  }
  return x;
}
"#;
        assert_eq!(
            cyclomatic(SupportedLanguage::TypeScript, src.as_bytes()),
            Some(4)
        );
    }

    #[test]
    fn trivial_function_has_complexity_one() {
        let src = "fn f() -> i32 { 42 }";
        assert_eq!(cyclomatic(SupportedLanguage::Rust, src.as_bytes()), Some(1));
    }

    // --- indentation fallback ---

    #[test]
    fn indentation_sums_nesting_depth() {
        // a(0) b(1) c(2) d(1) e(0) → 0+1+2+1+0 = 4.
        let src = "a\n  b\n    c\n  d\ne\n";
        assert_eq!(indentation_complexity(src.as_bytes()), 4);
    }

    #[test]
    fn indentation_ignores_blank_lines() {
        let src = "a\n\n  b\n\n";
        assert_eq!(indentation_complexity(src.as_bytes()), 1);
    }

    #[test]
    fn file_complexity_uses_fallback_for_unknown_ext() {
        let fc = file_complexity("notes.txt", b"a\n  b\n    c\n");
        assert_eq!(fc.method, ComplexityMethod::Indentation);
        assert_eq!(fc.language, None);
        assert_eq!(fc.value, 3.0); // 0+1+2
        assert_eq!(fc.loc, 3);
    }

    #[test]
    fn file_complexity_uses_ast_for_supported() {
        let fc = file_complexity("m.rs", b"fn f(x:i32){ if x>0 { } }");
        assert_eq!(fc.method, ComplexityMethod::Cyclomatic);
        assert_eq!(fc.language, Some(SupportedLanguage::Rust));
        assert_eq!(fc.value, 2.0); // 1 + один if
    }

    #[test]
    fn per_loc_normalizes() {
        let fc = file_complexity("m.rs", b"fn f(){}");
        assert!(fc.per_loc > 0.0);
        assert_eq!(fc.value, 1.0);
    }
}

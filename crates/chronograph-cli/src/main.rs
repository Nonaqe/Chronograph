//! Бинарь `chronograph`.
//!
//! Этап 0: `analyze` (кэш). Этап 1: `hotspots` (таблица в терминал). Без UI/графики
//! (правило 1 CLAUDE.md); `report`/`coupling` — следующие этапы.

use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(
    name = "chronograph",
    version,
    about = "Аналитика эволюции git-репозиториев"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Полный/инкрементальный анализ истории; строит кэш `.chronograph/cache.duckdb`.
    Analyze(commands::analyze::AnalyzeArgs),
    /// Топ hotspots (churn × complexity) в терминал.
    Hotspots(commands::hotspots::HotspotsArgs),
    /// Топ change-coupling пар (файлы, меняющиеся вместе) в терминал.
    Coupling(commands::coupling::CouplingArgs),
    /// Риск концентрации знаний (bus factor) по файлам в терминал.
    Knowledge(commands::knowledge::KnowledgeArgs),
    /// Распределение возраста строк (code age / stability) по файлам в терминал.
    Age(commands::age::AgeArgs),
    /// Self-contained HTML-репорт (Overview + Hotspots + Coupling + Knowledge).
    Report(commands::report::ReportArgs),
    /// Детерминированный JSON-экспорт метрик + потока событий (для Web UI/пайплайнов).
    Export(commands::export::ExportArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyze(args) => commands::analyze::run(args),
        Command::Hotspots(args) => commands::hotspots::run(args),
        Command::Coupling(args) => commands::coupling::run(args),
        Command::Knowledge(args) => commands::knowledge::run(args),
        Command::Age(args) => commands::age::run(args),
        Command::Report(args) => commands::report::run(args),
        Command::Export(args) => commands::export::run(args),
    }
}

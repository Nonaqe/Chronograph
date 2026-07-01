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
    /// Self-contained HTML-репорт (Overview + Hotspots treemap + Coupling).
    Report(commands::report::ReportArgs),
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Analyze(args) => commands::analyze::run(args),
        Command::Hotspots(args) => commands::hotspots::run(args),
        Command::Coupling(args) => commands::coupling::run(args),
        Command::Report(args) => commands::report::run(args),
    }
}

use std::time::Duration;

use clap::{Parser, Subcommand};

use gpui_kit::{App, AsyncApp, application, assets};
use serde::{Deserialize, Serialize};
use tokio::sync::watch::{self, Sender};
use tracing_subscriber::EnvFilter;

use crate::breaks::{
    ActiveState, BreakConfig, BreakRunner, BreakType, run_break, start_break_scheduler,
};

mod breaks;

#[derive(Copy, Clone, Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Maximum level of log messages
    #[arg(long)]
    level: Option<tracing::Level>,

    #[command(subcommand)]
    command: Option<CliCommand>,
}

impl Args {
    fn activate_break_system(&self) -> bool {
        Self::command_none_or_resume(self.command)
    }

    fn command_none_or_resume(command: Option<CliCommand>) -> bool {
        command.is_none_or(|command| matches!(command, CliCommand::Resume))
    }
}

#[derive(Copy, Clone, Subcommand, Debug, Serialize, Deserialize)]
enum CliCommand {
    Quit,
    Pause,
    Resume,
    /// Run a break manually
    RunBreak {
        /// Which break type to run
        #[arg(value_enum, default_value_t = BreakType::Short)]
        break_type: BreakType,
    },
    Settings,
}

async fn handle_command(cx: &mut AsyncApp, command: CliCommand, running_tx: Sender<bool>) {
    match command {
        CliCommand::Pause => _ = running_tx.send(false),
        CliCommand::Resume => _ = running_tx.send(true),
        CliCommand::RunBreak { break_type } => {
            run_break(cx, break_type).await;
        }
        _ => {}
    }
}

fn main() {
    let args = Args::parse();

    let level: tracing::Level = args.level.unwrap_or(tracing::Level::DEBUG);
    let env_filter = format!("error,{}={}", env!("CARGO_CRATE_NAME"), level);
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(env_filter))
        .init();

    let activate_break_system = args.activate_break_system();
    let (running_tx, running_rx) = watch::channel(activate_break_system);
    let active_state = ActiveState::new(running_rx);

    application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            let break_config = BreakConfig::new();
            cx.set_global(BreakRunner::new());
            start_break_scheduler(cx, break_config, active_state).detach();
            args.command.map(|command| {
                cx.spawn(async move |cx| {
                    handle_command(cx, command, running_tx).await;
                })
                .detach();
            });
        });
}

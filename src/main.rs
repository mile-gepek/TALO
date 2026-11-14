use clap::{Parser, Subcommand};

use gpui_kit::{App, application, assets};
use serde::{Deserialize, Serialize};
use tokio::sync::watch::{self};
use tracing_subscriber::EnvFilter;

use crate::{
    breaks::{ActiveState, BreakConfig, BreakRunner, BreakType, start_break_scheduler},
    ipc::spawn_socket_watcher,
};

mod breaks;
mod ipc;

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
pub enum CliCommand {
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

            cx.spawn(async move |cx| spawn_socket_watcher(cx, args.command, running_tx).await)
                .detach();
        });
}

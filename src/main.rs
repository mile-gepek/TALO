use clap::{Parser, Subcommand};

use gpui_kit::{App, AsyncApp, BorrowAppContext, application, assets};
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::{
    breaks::{BreakConfig, BreakRunner, BreakType, pause, resume, run_break_forced},
    ipc::{IpcRole, claim_or_forward, default_socket_path, spawn_socket_watcher},
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
    fn run_on_startup(&self) -> bool {
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

impl CliCommand {
    pub fn handle_command(self, cx: &mut AsyncApp) {
        match self {
            Self::Pause => pause(cx),
            Self::Resume => resume(cx),
            Self::RunBreak { break_type } => {
                run_break_forced(cx, break_type);
            }
            Self::Quit => cx.update(|cx| cx.quit()),
            Self::Settings => {
                todo!("Open settings screen");
            }
        }
    }
}

fn main() {
    let args = Args::parse();

    let level: tracing::Level = args.level.unwrap_or(tracing::Level::DEBUG);
    let env_filter = format!("error,{}={}", env!("CARGO_CRATE_NAME"), level);
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(env_filter))
        .init();

    let run_on_startup = args.run_on_startup();

    let Ok(IpcRole::Primary(listener)) =
        claim_or_forward(default_socket_path().unwrap(), args.command)
    else {
        return;
    };

    application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);

            let Some(mut ipc_command_rx) = spawn_socket_watcher(listener) else {
                tracing::debug!("Secondary instance, quitting");
                cx.quit();
                return;
            };

            let break_config = BreakConfig::new();
            let runner = BreakRunner::start(run_on_startup, break_config, cx);
            cx.set_global(runner);

            cx.on_window_closed(|cx, window_id| {
                cx.update_global::<BreakRunner, _>(|runner, cx| {
                    runner.on_window_close(cx, window_id)
                })
            })
            .detach();

            cx.spawn(async move |cx| {
                args.command.map(|command| command.handle_command(cx));

                while let Some(ipc_command) = ipc_command_rx.recv().await {
                    tracing::debug!("Got IPC command: {ipc_command:?}");
                    ipc_command.handle_command(cx);
                }
            })
            .detach();
        });
}

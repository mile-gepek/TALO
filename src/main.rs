use crate::{breaks::SchedulerEvent, config::BreakConfig, settings::SettingsGlobal};
use clap::{Parser, Subcommand};

use gpui_kit::{App, AsyncApp, application, assets, px, rgb};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedSender, error::SendError};
use tracing_subscriber::EnvFilter;

use crate::{
    breaks::{BreakScheduler, BreakType, start_break_scheduler},
    ipc::{IpcRole, claim_or_forward, default_socket_path, spawn_socket_watcher},
    tray::tray_listener,
};

mod breaks;
mod config;
mod ipc;
mod settings;
mod tray;

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
    pub fn handle(
        self,
        cx: &mut AsyncApp,
        event_sender: &UnboundedSender<SchedulerEvent>,
    ) -> Result<(), SendError<SchedulerEvent>> {
        let event = match self {
            Self::Pause => SchedulerEvent::Pause,
            Self::Resume => SchedulerEvent::Resume,
            Self::RunBreak { break_type } => SchedulerEvent::RunBreakForced(break_type),
            Self::Quit => {
                cx.update(|cx| cx.quit());
                return Ok(());
            }
            Self::Settings => {
                cx.update_global::<SettingsGlobal, _>(|settings, cx| settings.open_or_focus(cx));
                return Ok(());
            }
        };
        event_sender.send(event)
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

    let (tray_sender, mut tray_receiver) = tray_listener(run_on_startup);

    application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .with_assets(assets::Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            {
                let theme = gpui_component::Theme::global_mut(cx);
                theme.radius = px(6.);
                theme.radius_lg = px(10.);
                theme.border = rgb(0x333333).into();
                theme.background = rgb(0x1c1c1c).into();
                theme.secondary = rgb(0x232323).into();
                theme.foreground = rgb(0xDADADA).into();
                theme.focus_ring = false;
            }
            cx.refresh_windows();

            let settings_global = SettingsGlobal::new();
            cx.set_global(settings_global);

            let Some(mut ipc_command_rx) = spawn_socket_watcher(listener) else {
                tracing::debug!("Secondary instance, quitting");
                cx.quit();
                return;
            };

            let (event_sender, event_reader) = tokio::sync::mpsc::unbounded_channel();

            let break_config = BreakConfig::new();
            let runner = BreakScheduler::new(break_config, run_on_startup, tray_sender);
            start_break_scheduler(runner, cx, event_reader).detach();

            let event_sender_clone = event_sender.clone();
            cx.spawn(async move |cx| {
                if let Some(command) = args.command
                    && let Err(err) = command.handle(cx, &event_sender_clone)
                {
                    tracing::debug!("Error handling initial CLI command: {err}")
                }

                while let Some(ipc_command) = ipc_command_rx.recv().await {
                    tracing::debug!("Got IPC command: {ipc_command:?}");
                    if let Err(err) = ipc_command.handle(cx, &event_sender_clone) {
                        tracing::debug!("Error handling IPC command: {err}")
                    }
                }
            })
            .detach();

            let event_sender_clone = event_sender.clone();
            cx.on_window_closed(move |_, window_id| {
                _ = event_sender_clone.send(breaks::SchedulerEvent::WindowClosed(window_id));
            })
            .detach();

            cx.spawn(async move |cx| {
                while let Some(tray_command) = tray_receiver.recv().await {
                    tracing::debug!("Got tray command: {tray_command:?}");
                    if let Err(err) = tray_command.handle(cx, &event_sender) {
                        tracing::debug!("Error handling tray command: {err}")
                    }
                }
            })
            .detach();
        });
}

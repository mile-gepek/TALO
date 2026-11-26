use clap::{Parser, Subcommand};

use gpui_kit::{
    App, AppContext, BorrowAppContext, Context, Entity, Global, application, assets::Assets, hsla,
    px,
};
use serde::{Deserialize, Serialize};
use tracing_subscriber::EnvFilter;

use crate::{
    breaks::{BreakService, BreakType},
    config::{ConfigService, load_config_or_default},
    ipc::{IpcRole, IpcService, claim_or_forward, default_socket_path},
    settings::SettingsGlobal,
    tray::TrayService,
};

mod breaks;
mod components;
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
        command.is_none_or(|command| {
            matches!(
                command,
                CliCommand::BreakServiceCommand(BreakServiceCommand::Resume)
            )
        })
    }
}

#[derive(Copy, Clone, Subcommand, Debug, Serialize, Deserialize)]
pub enum CliCommand {
    Quit,
    Settings,
    ReloadConfig,
    #[command(flatten)]
    BreakServiceCommand(BreakServiceCommand),
}

#[derive(Copy, Clone, Subcommand, Debug, Serialize, Deserialize)]
pub enum BreakServiceCommand {
    Pause,
    Resume,
    /// Run a break manually
    RunBreak {
        /// Which break type to run
        #[arg(value_enum, default_value_t = BreakType::Short)]
        break_type: BreakType,
    },
}

struct TaloInner {
    break_service: Entity<BreakService>,
    _ipc_service: Entity<IpcService>,
    _tray_service: Entity<TrayService>,
    config_service: Entity<ConfigService>,
}

impl TaloInner {
    fn handle_command(&mut self, command: CliCommand, cx: &mut Context<Self>) {
        match command {
            CliCommand::Quit => cx.quit(),
            CliCommand::BreakServiceCommand(break_service_command) => self
                .break_service
                .read(cx)
                .handle_command(break_service_command),
            CliCommand::Settings => {
                cx.update_global::<SettingsGlobal, _>(|settings_global, cx| {
                    settings_global.open_or_focus(cx, self.config_service.downgrade())
                });
            }
            command => todo!("Implement {command:?} handling"),
        }
    }
}

struct Talo {
    _inner: Entity<TaloInner>,
}

impl Global for Talo {}

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

    let config = load_config_or_default();

    application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .with_assets(Assets)
        .run(move |cx: &mut App| {
            gpui_component::init(cx);
            gpui_component::Theme::change(gpui_component::ThemeMode::Dark, None, cx);
            gpui_component::Theme::update(cx, |theme| {
                theme.radius = px(6.);
                theme.radius_lg = px(10.);
                theme.border = hsla(0., 0., 0.25, 1.);
                theme.background = hsla(0.5, 0.05, 0.1, 1.);
                theme.focus_ring = false;
            });

            let settings_global = SettingsGlobal::new();
            cx.set_global(settings_global);

            let break_service = BreakService::start(run_on_startup, config.breaks, cx);
            let ipc_service = IpcService::start_with_listener(listener, args.command, cx);
            let tray_service = TrayService::start(run_on_startup, cx);
            let config_service = cx.new(|_| ConfigService);

            break_service.update(cx, |_, cx| {
                cx.subscribe(
                    &config_service,
                    |break_service, _, config_event, _| match config_event {
                        config::ConfigEvent::Changed(new_config) => {
                            let break_config = new_config.breaks;
                            break_service.change_config(break_config);
                        }
                    },
                )
                .detach();
            });

            tray_service.update(cx, |_, cx| {
                cx.subscribe(&break_service, |tray_service, _, tray_messages, _| {
                    tray_service.handle_messages(tray_messages.clone());
                })
                .detach();
            });

            let break_service_weak = break_service.downgrade();
            cx.on_window_closed(move |cx, window_id| {
                _ = break_service_weak.update(cx, |break_service, _| {
                    break_service.on_window_closed(window_id);
                });
            })
            .detach();

            let talo = cx.new(|cx| {
                cx.subscribe(&ipc_service, |talo: &mut TaloInner, _, command, cx| {
                    talo.handle_command(*command, cx)
                })
                .detach();

                cx.subscribe(&tray_service, |talo: &mut TaloInner, _, command, cx| {
                    talo.handle_command(*command, cx)
                })
                .detach();

                TaloInner {
                    break_service,
                    _ipc_service: ipc_service,
                    _tray_service: tray_service,
                    config_service,
                }
            });

            cx.set_global(Talo { _inner: talo });
        });
}

use std::{
    sync::mpsc::{Sender, channel},
    time::Duration,
};

use crate::{BreakServiceCommand, BreakType, CliCommand};

use gpui_kit::{App, AppContext, Entity, EventEmitter, Task};
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use tray_icon::{
    TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem, Submenu},
};

const ID_STATUS: &str = "status";
const ID_BREAK_SHORT: &str = "short break";
const ID_BREAK_LONG: &str = "long break";
const ID_BREAK_INF: &str = "infinite break";
const ID_OPEN_SETTINGS: &str = "open settings";
const ID_PAUSE: &str = "pause";
const ID_RESUME: &str = "resume";
const ID_QUIT: &str = "quit";

/// Message sent TO the tray handler, to update the menu.
#[derive(Copy, Clone, Debug)]
pub enum TrayMessage {
    Paused,
    Resumed,
    Scheduled(chrono::NaiveTime, BreakType),
    BreakFinished,
}

fn create_tray(initial_running: bool) -> Sender<Vec<TrayMessage>> {
    let (tray_tx, tray_rx) = channel();

    std::thread::spawn(move || {
        #[cfg(target_os = "linux")]
        gtk::init().unwrap();

        let tray_menu = Menu::new();

        let status_text = if initial_running { "Idle" } else { "Paused" };
        let status_item = &MenuItem::with_id(ID_STATUS, status_text, false, None);

        _ = tray_menu.append(status_item);

        let submenu = Submenu::new("Run a break", true);
        _ = submenu.append_items(&[
            &MenuItem::with_id(ID_BREAK_SHORT, "Short break", true, None),
            &MenuItem::with_id(ID_BREAK_LONG, "Long break", true, None),
            &MenuItem::with_id(ID_BREAK_INF, "Infinite break", true, None),
        ]);

        _ = tray_menu.append(&submenu);

        let pause_item = &MenuItem::with_id(ID_PAUSE, "Pause", initial_running, None);
        let resume_item = &MenuItem::with_id(ID_RESUME, "Resume", !initial_running, None);

        _ = tray_menu.append_items(&[
            &MenuItem::with_id(ID_OPEN_SETTINGS, "Open settings", true, None),
            pause_item,
            resume_item,
            &MenuItem::with_id(ID_QUIT, "Quit", true, None),
        ]);
        let _tray_icon = TrayIconBuilder::new()
            .with_tooltip("TALO - Take a load off.")
            .with_menu(Box::new(tray_menu))
            .build()
            .unwrap();

        loop {
            #[cfg(target_os = "linux")]
            {
                if gtk::events_pending() {
                    gtk::main_iteration();
                }
            }
            let timeout = Duration::from_millis(50);
            if let Ok(messages) = tray_rx.recv_timeout(timeout) {
                for message in messages {
                    match message {
                        TrayMessage::Paused => {
                            pause_item.set_enabled(false);
                            resume_item.set_enabled(true);

                            status_item.set_text("Idle")
                        }
                        TrayMessage::Resumed => {
                            pause_item.set_enabled(true);
                            resume_item.set_enabled(false);
                        }
                        TrayMessage::Scheduled(scheduled_time, break_type) => {
                            let formatted = scheduled_time.format("%H:%M:%S");
                            let status_text = format!("{break_type:?} break at {formatted}");
                            status_item.set_text(status_text);
                        }
                        TrayMessage::BreakFinished => {
                            status_item.set_text("Idle");
                        }
                    }
                }
            }
        }
    });

    tray_tx
}

pub fn tray_listener(
    initial_running: bool,
) -> (Sender<Vec<TrayMessage>>, UnboundedReceiver<CliCommand>) {
    let tray_sender = create_tray(initial_running);

    let (tray_event_tx, tray_event_rx) = unbounded_channel();

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        let event = match event.id.0.as_str() {
            ID_BREAK_SHORT => CliCommand::BreakServiceCommand(BreakServiceCommand::RunBreak {
                break_type: BreakType::Short,
            }),
            ID_BREAK_LONG => CliCommand::BreakServiceCommand(BreakServiceCommand::RunBreak {
                break_type: BreakType::Long,
            }),
            ID_BREAK_INF => CliCommand::BreakServiceCommand(BreakServiceCommand::RunBreak {
                break_type: BreakType::Infinite,
            }),
            ID_PAUSE => CliCommand::BreakServiceCommand(BreakServiceCommand::Pause),
            ID_RESUME => CliCommand::BreakServiceCommand(BreakServiceCommand::Resume),
            ID_OPEN_SETTINGS => CliCommand::Settings,
            ID_QUIT => CliCommand::Quit,
            id => {
                tracing::warn!("Event from unmatched tray item: {id}");
                return;
            }
        };
        _ = tray_event_tx.send(event);
    }));

    (tray_sender, tray_event_rx)
}

pub struct TrayService {
    sender: Sender<Vec<TrayMessage>>,
    _task: Task<()>,
}

impl EventEmitter<CliCommand> for TrayService {}

impl TrayService {
    pub fn start(initial_running: bool, cx: &mut App) -> Entity<Self> {
        cx.new(|cx| {
            let (tray_sender, mut tray_event_rx) = tray_listener(initial_running);

            let task = cx.spawn(async move |tray_service, cx| {
                while let Some(tray_command) = tray_event_rx.recv().await {
                    if tray_service
                        .update(cx, |_, cx| {
                            cx.emit(tray_command);
                        })
                        .is_err()
                    {
                        tracing::debug!("Tray service dropped before task")
                    }
                }
            });

            Self {
                sender: tray_sender,
                _task: task,
            }
        })
    }

    pub fn handle_messages(&self, messages: Vec<TrayMessage>) {
        _ = self.sender.send(messages);
    }
}

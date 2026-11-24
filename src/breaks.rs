use std::time::{Duration, Instant};

use clap::ValueEnum;

use gpui_component::{Sizable, StyledExt, button::ButtonVariants};
use gpui_kit::{
    Animation, AnimationExt, App, AsyncApp, Context, Entity, EventEmitter, Task, TitlebarOptions,
    Window, WindowHandle, WindowId, WindowOptions, component::button::Button, div,
    layer_shell::LayerShellOptions, prelude::*, px, relative, rems, rgb,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};

use crate::{
    BreakServiceCommand,
    config::{AfterResumeBehavior, BreakConfig, CONFIG, Config},
    tray::TrayMessage,
};

struct BreakView {
    started_at: Instant,
    break_type: BreakType,
    /// A task for the break to tick the shown timer and progress bar
    _ticker: Task<()>,
}

impl BreakView {
    pub fn new(cx: &mut Context<Self>, break_type: BreakType) -> Self {
        let handle = cx.spawn(async move |this, cx| {
            loop {
                // break screen timer ticks every second, progress bar uses an Animation.
                cx.background_executor().timer(Duration::from_secs(1)).await;
                this.update(cx, |_, cx| {
                    cx.notify();
                })
                .ok();
            }
        });
        BreakView {
            started_at: Instant::now(),
            break_type,
            _ticker: handle,
        }
    }
}

impl Render for BreakView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let break_text = self.break_type.text();
        let break_text = div()
            .child(break_text)
            .text_size(rems(2.5))
            .font_bold()
            .font_family("Liberation Sans");
        let button_label = if matches!(self.break_type, BreakType::Infinite) {
            "End break"
        } else {
            "Skip break"
        };
        let button = Button::new("skip-button")
            .px(px(18.))
            .py_2p5()
            .text_center()
            .with_size(px(0.))
            .text_color(rgb(0xEDEDED))
            .border_2()
            .border_color(rgb(0xEDEDED))
            .bg(gpui_kit::transparent_black())
            .rounded_full()
            .ghost()
            .child(div().text_size(rems(1.1)).child(button_label))
            .on_click(|_, window, _| {
                window.remove_window();
            });

        let mut center_div = div()
            .flex()
            .flex_col()
            .m_auto()
            .gap_12()
            .w(rems(20.))
            .justify_center()
            .items_center()
            .child(break_text);

        if !matches!(self.break_type, BreakType::Infinite) {
            let end_at = self.started_at + self.break_type.duration();
            let time_remaining = end_at.duration_since(Instant::now());
            let seconds_remaining = time_remaining.as_secs_f32();
            let minutes = (seconds_remaining.ceil() / 60.).round();
            let seconds = (seconds_remaining.ceil() % 60.).round();
            let time_remaining_text = div()
                .text_size(rems(1.2))
                .child(format!("{minutes:02}:{seconds:02}"));
            let progress_bar_animation = Animation::new(self.break_type.duration());
            let progress_bar = div()
                .w_full()
                .h(rems(0.8))
                .rounded_full()
                .bg(rgb(0x888888))
                .overflow_hidden()
                .child(
                    div()
                        .overflow_hidden()
                        .h_full()
                        .bg(rgb(0xFFFFFF))
                        .rounded_full()
                        .with_animation(
                            "break-progress-bar-animation",
                            progress_bar_animation,
                            |progress_bar, fraction| progress_bar.w(relative(fraction)),
                        ),
                );

            let progress_wrapper = div()
                .w_full()
                .flex()
                .flex_col()
                .items_center()
                .gap_2()
                .child(time_remaining_text)
                .child(progress_bar);

            center_div = center_div.child(progress_wrapper)
        }

        center_div = center_div.child(button);

        div()
            .h_full()
            .w_full()
            .text_color(rgb(0xEDEDED))
            .bg(gpui_kit::transparent_black().alpha(0.8))
            .content_center()
            .child(center_div)
    }
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, ValueEnum)]
pub enum BreakType {
    Long,
    Short,
    Infinite,
}

impl BreakType {
    fn duration(&self) -> Duration {
        match self {
            BreakType::Short => Duration::from_secs(5),
            BreakType::Long => Duration::from_secs(15),
            BreakType::Infinite => Duration::MAX,
        }
    }

    fn text(&self) -> String {
        match self {
            BreakType::Short => "Short break",
            BreakType::Long => "Long break",
            BreakType::Infinite => "Take a load off.",
        }
        .to_string()
    }
}

#[derive(Copy, Clone)]
struct ScheduledBreak {
    run_at: Instant,
    break_type: BreakType,
}

impl ScheduledBreak {
    fn new(run_at: Instant, break_type: BreakType) -> Self {
        Self { run_at, break_type }
    }

    fn schedule(config: &BreakConfig, short_breaks_run: usize) -> Self {
        let break_type = if short_breaks_run >= config.long_break_interval {
            BreakType::Long
        } else {
            BreakType::Short
        };
        let run_at = Instant::now() + config.short_break_interval;
        ScheduledBreak::new(run_at, break_type)
    }

    fn schedule_for(config: &BreakConfig, short_breaks_run: usize, deadline: Instant) -> Self {
        let break_type = if short_breaks_run >= config.long_break_interval {
            BreakType::Long
        } else {
            BreakType::Short
        };
        ScheduledBreak::new(deadline, break_type)
    }
}

#[derive(Debug)]
struct RunningBreak {
    window_handle: WindowHandle<BreakView>,
    _window_close_task: Option<Task<()>>,
}

impl RunningBreak {
    fn close(&self, cx: &mut AsyncApp) {
        _ = self
            .window_handle
            .update(cx, |_, window, _| window.remove_window());
    }

    fn open(cx: &mut AsyncApp, break_type: BreakType) -> RunningBreak {
        tracing::debug!("Running break");
        let window_options = WindowOptions {
            focus: true,
            titlebar: Some(TitlebarOptions {
                title: Some("Talo".into()),
                ..Default::default()
            }),
            kind: gpui_kit::WindowKind::LayerShell(LayerShellOptions {
                exclusive_zone: Some(px(-1.)),
                anchor: gpui_kit::layer_shell::Anchor::all(),
                layer: gpui_kit::layer_shell::Layer::Overlay,
                namespace: "talo_break_screen".to_string(),
                ..Default::default()
            }),
            window_background: gpui_kit::WindowBackgroundAppearance::Blurred,
            app_id: Some("talo".to_string()),
            ..Default::default()
        };
        let window_handle = cx
            .open_window(window_options, |_, cx| {
                cx.new(|cx| BreakView::new(cx, break_type))
            })
            .unwrap();

        let close_task = if matches!(break_type, BreakType::Infinite) {
            None
        } else {
            Some(cx.spawn(async move |cx| {
                cx.background_executor().timer(break_type.duration()).await;
                _ = window_handle.update(cx, |_, window, _| window.remove_window());
            }))
        };
        Self {
            window_handle,
            _window_close_task: close_task,
        }
    }
}

enum BreakKind {
    Scheduled(ScheduledBreak),
    Running(RunningBreak),
}

impl BreakKind {
    fn is_running_for(&self, window_id: &WindowId) -> bool {
        match self {
            BreakKind::Running(running_break) => {
                &running_break.window_handle.window_id() == window_id
            }
            _ => false,
        }
    }

    fn into_running(self) -> Option<RunningBreak> {
        match self {
            Self::Running(running_break) => Some(running_break),
            _ => None,
        }
    }
}

enum ActiveState {
    Paused(Instant),
    Running,
}

impl ActiveState {
    fn new(running: bool) -> Self {
        if running {
            Self::Running
        } else {
            Self::Paused(Instant::now())
        }
    }

    fn running(&mut self) -> bool {
        matches!(self, Self::Running)
    }
}

#[derive(Debug)]
enum BreakServiceEvent {
    Command(BreakServiceCommand),
    ReloadConfig,
    WindowClosed(WindowId),
}

/// Side-effects of the break system.
#[derive(Debug)]
enum Effect {
    OpenBreak(BreakType),
    TrayMessage(Vec<TrayMessage>),
}

struct BreakServiceState {
    break_state: Option<BreakKind>,
    short_breaks_run: usize,
    active_state: ActiveState,
    config: BreakConfig,
}

impl BreakServiceState {
    pub fn new(run_on_startup: bool) -> Self {
        let config = CONFIG.load().breaks;
        Self {
            short_breaks_run: 0,
            break_state: None,
            active_state: ActiveState::new(run_on_startup),
            config,
        }
    }

    fn handle(&mut self, event: &BreakServiceEvent) -> Option<Effect> {
        match event {
            BreakServiceEvent::Command(command) => match command {
                BreakServiceCommand::Pause => {
                    return Some(Effect::TrayMessage(self.pause().into_iter().collect()));
                }
                BreakServiceCommand::Resume => return Some(Effect::TrayMessage(self.resume())),
                BreakServiceCommand::RunBreak { break_type } => {
                    return Some(Effect::OpenBreak(*break_type));
                }
            },
            BreakServiceEvent::WindowClosed(window_id) => {
                return self
                    .on_window_close(window_id)
                    .map(|tray_message| Effect::TrayMessage(vec![tray_message]));
            }
            BreakServiceEvent::ReloadConfig => {
                self.reload_config();
            }
        }
        None
    }

    fn pause(&mut self) -> Option<TrayMessage> {
        if matches!(self.active_state, ActiveState::Paused(..)) {
            return None;
        }
        tracing::debug!("Pausing");
        self.active_state = ActiveState::Paused(Instant::now());
        Some(TrayMessage::Paused)
    }

    fn resume(&mut self) -> Vec<TrayMessage> {
        let mut messages = Vec::with_capacity(2);
        let ActiveState::Paused(paused_at) = self.active_state else {
            return messages;
        };
        tracing::debug!("Resuming");
        self.active_state = ActiveState::Running;
        messages.push(TrayMessage::Resumed);
        let Some(BreakKind::Scheduled(scheduled_break)) = self.break_state.as_ref() else {
            messages.extend(self.schedule_new());
            return messages;
        };
        let scheduled_message = match self.config.after_resume_config.behavior {
            AfterResumeBehavior::Reschedule => self.schedule_new(),
            AfterResumeBehavior::KeepScheduledBreak => {
                let after_resume_config = &self.config.after_resume_config;
                let delay = scheduled_break
                    .run_at
                    .duration_since(paused_at)
                    .max(after_resume_config.min_wait_after_resume);
                let deadline = Instant::now() + delay;
                self.schedule_for(deadline)
            }
        };
        messages.extend(scheduled_message);
        messages
    }

    fn on_window_close(&mut self, window_id: &WindowId) -> Option<TrayMessage> {
        self.break_state
            .take_if(|break_state| break_state.is_running_for(window_id))?;
        match self.active_state {
            ActiveState::Running => self.schedule_new(),
            _ => Some(TrayMessage::BreakFinished),
        }
    }

    fn schedule_for(&mut self, deadline: Instant) -> Option<TrayMessage> {
        if matches!(self.break_state, Some(BreakKind::Running(..))) {
            return None;
        }
        let scheduled_break =
            ScheduledBreak::schedule_for(&self.config, self.short_breaks_run, deadline);
        self.break_state = Some(BreakKind::Scheduled(scheduled_break));
        Some(self.break_scheduled_message(scheduled_break))
    }

    fn schedule_new(&mut self) -> Option<TrayMessage> {
        if matches!(self.break_state, Some(BreakKind::Running(..))) {
            return None;
        }
        let scheduled_break = ScheduledBreak::schedule(&self.config, self.short_breaks_run);
        self.break_state = Some(BreakKind::Scheduled(scheduled_break));
        Some(self.break_scheduled_message(scheduled_break))
    }

    fn break_scheduled_message(&mut self, scheduled_break: ScheduledBreak) -> TrayMessage {
        let now = Instant::now();
        let time_until_break = scheduled_break.run_at.duration_since(now);
        let naive_time = chrono::Local::now().time() + time_until_break;

        let time_formatted = naive_time.format("%H:%M:%S");
        tracing::info!("Scheduling break for {time_formatted}");

        TrayMessage::Scheduled(naive_time, scheduled_break.break_type)
    }

    fn open_break(&mut self, cx: &mut AsyncApp, break_type: BreakType) {
        if let Some(BreakKind::Running(running_break)) = &self.break_state {
            running_break.close(cx);
        }
        let running_break = RunningBreak::open(cx, break_type);
        self.break_state = Some(BreakKind::Running(running_break));
    }

    fn on_scheduled_break_timer_finished(&mut self, cx: &mut AsyncApp) {
        let Some(BreakKind::Scheduled(scheduled_break)) =
            self.break_state.take_if(|_| self.active_state.running())
        else {
            return;
        };
        self.open_break(cx, scheduled_break.break_type);
    }

    fn reload_config(&mut self) {
        match Config::reload() {
            Ok(config) => {
                self.config = config.breaks;
                // TODO: diff the configs and reschedule if needed.
            }
            Err(err) => tracing::error!("Error reloading config: '{err}'"),
        }
    }
}

pub struct BreakService {
    event_sender: UnboundedSender<BreakServiceEvent>,
    _task: Task<()>,
}

impl EventEmitter<Vec<TrayMessage>> for BreakService {}

impl BreakService {
    pub fn start(run_on_startup: bool, cx: &mut App) -> Entity<Self> {
        let (event_sender, mut event_reader) = unbounded_channel();

        cx.new(|cx| {
            let task = cx.spawn(async move |service, cx| {
                let mut scheduler = BreakServiceState::new(run_on_startup);
                let tray_message = if run_on_startup {
                    scheduler
                        .schedule_new()
                        .expect("No running break on startup, so this is always Some")
                } else {
                    TrayMessage::Paused
                };
                if service
                    .update(cx, |_, cx| cx.emit(vec![tray_message]))
                    .is_err()
                {
                    tracing::warn!("Break service entity dropped, but task wasn't");
                    return;
                };

                loop {
                    let event = match &scheduler.break_state {
                        Some(BreakKind::Scheduled(scheduled_break)) => {
                            let now = Instant::now();
                            let time_sleep = scheduled_break.run_at.duration_since(now);
                            tokio::select! {
                                event = event_reader.recv() => event,
                                _ = {
                                    cx.background_executor().timer(time_sleep)
                                },
                                if scheduler.active_state.running() => {
                                    scheduler.on_scheduled_break_timer_finished(cx);
                                    continue;
                                }
                            }
                        }
                        _ => event_reader.recv().await,
                    };
                    let Some(event) = event else { return };
                    if let Some(effect) = scheduler.handle(&event) {
                        tracing::debug!("Break service effect: {effect:?}");
                        match effect {
                            Effect::OpenBreak(break_type) => scheduler.open_break(cx, break_type),
                            Effect::TrayMessage(tray_messages) => {
                                if tray_messages.is_empty() {
                                    return;
                                }
                                if service.update(cx, |_, cx| cx.emit(tray_messages)).is_err() {
                                    tracing::warn!("Break service entity dropped, but task wasn't");
                                    return;
                                };
                            }
                        }
                    }
                }
            });

            Self {
                event_sender,
                _task: task,
            }
        })
    }

    pub fn handle_command(&self, command: BreakServiceCommand) {
        let event = BreakServiceEvent::Command(command);
        _ = self.event_sender.send(event);
    }
}

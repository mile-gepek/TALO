use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use clap::ValueEnum;

use gpui_component::{Sizable, StyledExt, button::ButtonVariants};
use gpui_kit::{
    Animation, AnimationExt, App, AsyncApp, Context, Global, Task, TitlebarOptions, Window,
    WindowHandle, WindowId, WindowOptions, component::button::Button, div,
    layer_shell::LayerShellOptions, prelude::*, px, relative, rems, rgb,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, error::SendError};

use crate::{CliCommand, tray::TrayMessage};

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

pub struct BreakConfig {
    break_interval: Duration,
    long_break_interval: usize,
}

impl BreakConfig {
    pub fn new() -> Self {
        BreakConfig {
            break_interval: Duration::from_secs(5),
            long_break_interval: 3,
        }
    }

    fn next_break(&self, short_breaks_run: usize) -> ScheduledBreak {
        let break_type = if short_breaks_run >= self.long_break_interval {
            BreakType::Long
        } else {
            BreakType::Short
        };
        let run_at = Instant::now() + self.break_interval;
        ScheduledBreak::new(run_at, break_type)
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
}

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

pub struct BreakScheduler {
    break_state: Option<BreakKind>,
    short_breaks_run: usize,
    running: bool,
    config: BreakConfig,
    tray_sender: Sender<TrayMessage>,
}

impl Global for BreakScheduler {}

#[derive(Debug)]
pub enum SchedulerEvent {
    RunBreakForced(BreakType),
    Pause,
    Resume,
    WindowClosed(WindowId),
}

/// Side-effects of the break system.
enum Action {
    OpenBreak(BreakType),
    CloseBreak(RunningBreak),
}

impl BreakScheduler {
    pub fn new(
        config: BreakConfig,
        run_on_startup: bool,
        tray_sender: Sender<TrayMessage>,
    ) -> Self {
        let mut instance = Self {
            short_breaks_run: 0,
            break_state: None,
            running: run_on_startup,
            config,
            tray_sender,
        };
        instance.schedule();
        instance
    }

    fn handle(&mut self, event: SchedulerEvent) -> Option<Action> {
        match event {
            SchedulerEvent::Pause => self.pause(),
            SchedulerEvent::Resume => self.resume(),
            SchedulerEvent::WindowClosed(window_id) => {
                return self.on_window_close(window_id);
            }
            SchedulerEvent::RunBreakForced(break_type) => {
                return Some(Action::OpenBreak(break_type));
            }
        }
        None
    }

    fn pause(&mut self) {
        tracing::debug!("Pausing");
        self.running = false;
        if matches!(self.break_state, Some(BreakKind::Scheduled(..))) {
            self.break_state = None;
        }
        _ = self.tray_sender.send(TrayMessage::ActiveStateChange(false));
    }

    fn resume(&mut self) {
        if self.running {
            return;
        }
        tracing::debug!("Resuming");
        self.running = true;
        self.schedule();
        _ = self.tray_sender.send(TrayMessage::ActiveStateChange(true));
    }

    fn on_window_close(&mut self, window_id: WindowId) -> Option<Action> {
        let Some(BreakKind::Running(running_break)) =
            self.break_state.take_if(|break_state| match break_state {
                BreakKind::Running(running_break) => {
                    running_break.window_handle.window_id() == window_id
                }
                _ => false,
            })
        else {
            return None;
        };
        _ = self.tray_sender.send(TrayMessage::BreakFinished);
        Some(Action::CloseBreak(running_break))
    }

    pub fn schedule(&mut self) {
        if matches!(self.break_state, Some(BreakKind::Running(..))) {
            return;
        }
        tracing::debug!("Scheduling break");
        let scheduled_break = self.config.next_break(self.short_breaks_run);
        self.break_state = Some(BreakKind::Scheduled(scheduled_break));

        let time_until_break = scheduled_break.run_at.elapsed();
        let naive_time = chrono::Local::now().time() + time_until_break;

        _ = self.tray_sender.send(TrayMessage::BreakScheduled(
            naive_time,
            scheduled_break.break_type,
        ));
    }

    fn open_break(&mut self, cx: &mut AsyncApp, break_type: BreakType) {
        let running_break = RunningBreak::open(cx, break_type);
        self.break_state = Some(BreakKind::Running(running_break));
    }

    fn scheduled_break_timer_finished(&mut self, cx: &mut AsyncApp) {
        let Some(BreakKind::Scheduled(scheduled_break)) = self.break_state else {
            return;
        };
        self.open_break(cx, scheduled_break.break_type);
    }
}

pub fn start_break_scheduler(
    mut scheduler: BreakScheduler,
    cx: &mut App,
    mut event_reader: UnboundedReceiver<SchedulerEvent>,
) -> Task<()> {
    cx.spawn(async move |cx| loop {
        let event = match &scheduler.break_state {
            Some(BreakKind::Scheduled(scheduled_break)) => tokio::select! {
                event = event_reader.recv() => event,
                _ = cx.background_executor().timer(scheduled_break.run_at.saturating_duration_since(Instant::now())) => {
                    scheduler.scheduled_break_timer_finished(cx);
                    continue;
                }
            },
            _ => event_reader.recv().await,
        };
        let Some(event) = event else { return };
        if let Some(action) = scheduler.handle(event) {
            match action {
                Action::CloseBreak(running_break) => running_break.close(cx),
                Action::OpenBreak(break_type) => scheduler.open_break(cx, break_type),
            }
        }
    })
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
                todo!("Open settings screen");
            }
        };
        event_sender.send(event)
    }
}

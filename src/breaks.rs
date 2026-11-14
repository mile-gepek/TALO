use std::time::{Duration, Instant};

use clap::ValueEnum;

use gpui_component::{Sizable, StyledExt};
use gpui_kit::{
    Animation, AnimationExt, App, AsyncApp, Context, Global, Length, Pixels, Task, Window,
    WindowHandle, WindowOptions, component::button::Button, div, prelude::*, px, rgb,
};
use serde::{Deserialize, Serialize};

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

    fn next_break(&self, short_breaks_run: usize) -> (Duration, BreakType) {
        let break_type = if short_breaks_run >= self.long_break_interval {
            BreakType::Long
        } else {
            BreakType::Short
        };
        (self.break_interval, break_type)
    }
}

struct RunningBreak {
    break_type: BreakType,
    window_handle: WindowHandle<BreakView>,
    _window_close_task: Option<Task<()>>,
}

impl RunningBreak {
    fn close(&self, cx: &mut AsyncApp) {
        _ = self
            .window_handle
            .update(cx, |_, window, _| window.remove_window());
    }
}

enum BreakKind {
    Scheduled(Task<()>),
    Running(RunningBreak),
}

impl BreakKind {
    fn running(
        window_handle: WindowHandle<BreakView>,
        break_type: BreakType,
        task: Option<Task<()>>,
    ) -> Self {
        Self::Running(RunningBreak {
            window_handle,
            break_type,
            _window_close_task: task,
        })
    }
}

pub struct BreakRunner {
    break_state: Option<BreakKind>,
    config: BreakConfig,
    short_breaks_run: usize,
    running: bool,
}

impl BreakRunner {
    pub fn start(run_on_startup: bool, config: BreakConfig, cx: &mut App) -> Self {
        let mut instance = Self {
            break_state: None,
            config,
            short_breaks_run: 0,
            running: run_on_startup,
        };
        if run_on_startup {
            instance.schedule_break(cx);
        }
        instance
    }

    fn pause(&mut self) {
        tracing::debug!("Pausing");
        if matches!(self.break_state, Some(BreakKind::Scheduled(..))) {
            tracing::debug!("Clearing scheduled break");
            self.break_state = None;
        }
        self.running = false;
    }

    fn resume(&mut self, cx: &mut App) {
        tracing::debug!("Resuming");
        if self.running {
            return;
        }
        self.running = true;
        if self.break_state.is_none() {
            self.schedule_break(cx);
        }
    }

    fn schedule_break(&mut self, cx: &mut App) {
        let (time_until_break, break_type) = self.config.next_break(self.short_breaks_run);
        tracing::debug!("Scheduled {break_type:?} break");
        let break_task = cx.spawn(async move |cx| {
            cx.background_executor().timer(time_until_break).await;
            Self::run_break(cx, break_type);
        });
        self.break_state = Some(BreakKind::Scheduled(break_task));
    }

    fn run_break_impl(&mut self, cx: &mut App, break_type: BreakType) {
        let _old_task = self.break_state.take();

        tracing::debug!("Running break");
        let window_handle = cx
            .open_window(
                WindowOptions {
                    ..Default::default()
                },
                |_, cx| cx.new(|cx| BreakView::new(cx, break_type)),
            )
            .unwrap();

        let close_task = if matches!(break_type, BreakType::Infinite) {
            None
        } else {
            Some(cx.spawn(async move |cx| {
                cx.background_executor().timer(break_type.duration()).await;
                _ = window_handle.update(cx, |_, window, _| window.remove_window());
            }))
        };
        self.break_state = Some(BreakKind::running(window_handle, break_type, close_task));
    }

    fn run_break_forced(cx: &mut AsyncApp, break_type: BreakType) {
        let running_break = cx.update_global::<BreakRunner, _>(|runner, cx| {
            let old = runner.take_running_break();
            runner.run_break_impl(cx, break_type);
            old
        });

        if let Some(running_break) = running_break {
            running_break.close(cx);
        }
    }

    fn run_break(cx: &mut AsyncApp, break_type: BreakType) {
        let running_break = cx.update_global::<BreakRunner, _>(|runner, cx| {
            let running_break = runner.take_running_break();
            if let Some(active_break) = &running_break {
                if !matches!(active_break.break_type, BreakType::Infinite) {
                    return None;
                }
            };
            runner.run_break_impl(cx, break_type);
            runner.short_breaks_run += 1;
            if runner.short_breaks_run > runner.config.long_break_interval {
                runner.short_breaks_run = 0;
            }
            running_break
        });

        if let Some(running_break) = running_break {
            running_break.close(cx);
        }
    }

    fn take_running_break(&mut self) -> Option<RunningBreak> {
        match self.break_state.take() {
            Some(BreakKind::Running(running_break)) => Some(running_break),
            _ => None,
        }
    }

    pub fn on_window_close(&mut self, cx: &mut App, window_id: WindowId) {
        let Some(BreakKind::Running(running_break)) = &self.break_state else {
            return;
        };
        if running_break.window_handle.window_id() != window_id {
            return;
        }
        self.break_state = None;
        if self.running {
            self.schedule_break(cx);
        }
    }
}

impl Global for BreakRunner {}

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

struct BreakView {
    started_at: Instant,
    break_type: BreakType,
    /// A handle for the break to tick the shown timer and progress bar,
    /// and eventually exit the break screen
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
        let break_text = div().child(break_text).text_3xl().font_bold();
        let button_label = if matches!(self.break_type, BreakType::Infinite) {
            "End break"
        } else {
            "Skip break"
        };
        let button = Button::new("skip-button")
            .px_6()
            .py_2()
            .text_center()
            .with_size(px(40.))
            .text_color(rgb(0xEDEDED))
            .border_2()
            .border_color(rgb(0xEDEDED))
            .bg(gpui_kit::transparent_black())
            .rounded(Pixels::from(9.))
            .label(button_label)
            .on_click(|_, window, _| {
                window.remove_window();
                cx.update_global::<BreakRunner, _>(|runner, cx| runner.close_and_schedule(cx))
            });

        let mut center_div = div()
            .flex()
            .flex_col()
            .m_auto()
            .gap_12()
            .w(gpui_kit::Length::Definite(
                gpui_kit::DefiniteLength::Absolute(gpui_kit::AbsoluteLength::Rems(gpui_kit::Rems(
                    20.,
                ))),
            ))
            // .h(gpui_kit::Length::Definite(
            //     gpui_kit::DefiniteLength::Absolute(gpui_kit::AbsoluteLength::Rems(gpui_kit::Rems(
            //         20.,
            //     ))),
            // ))
            .justify_center()
            .items_center()
            .child(break_text);

        if !matches!(self.break_type, BreakType::Infinite) {
            let end_at = self.started_at + self.break_type.duration();
            let time_remaining = end_at.duration_since(Instant::now());
            let seconds_remaining = time_remaining.as_secs_f32();
            let minutes = (seconds_remaining.ceil() / 60.).round();
            let seconds = (seconds_remaining.ceil() % 60.).round();
            let time_remaining_text = format!("{minutes:02}:{seconds:02}");
            let progress_bar_animation = Animation::new(self.break_type.duration());
            let progress_bar = div()
                .w_full()
                .h_10()
                .p_1()
                .border_2()
                .border_color(rgb(0xEDEDED))
                .rounded(Pixels::from(9.))
                .bg(gpui_kit::transparent_black())
                .overflow_hidden()
                .child(
                    div()
                        .rounded(Pixels::from(5.))
                        .h_full()
                        .bg(rgb(0xFFFFFF))
                        .with_animation(
                            "break-progress-bar-animation",
                            progress_bar_animation,
                            |progress_bar, progress_bar_value| {
                                progress_bar.w(Length::Definite(
                                    gpui_kit::DefiniteLength::Fraction(progress_bar_value),
                                ))
                            },
                        ),
                );

            center_div = center_div.child(time_remaining_text).child(progress_bar)
        }

        center_div = center_div.child(button);

        div()
            .h_full()
            .w_full()
            .text_color(rgb(0xEDEDED))
            .bg(rgb(0x505050))
            .content_center()
            .child(center_div)
    }
}

pub fn run_break_forced(cx: &mut AsyncApp, break_type: BreakType) {
    BreakRunner::run_break_forced(cx, break_type);
}

pub fn pause(cx: &mut AsyncApp) {
    cx.update_global::<BreakRunner, _>(|runner, _| runner.pause())
}

pub fn resume(cx: &mut AsyncApp) {
    cx.update_global::<BreakRunner, _>(|runner, cx| runner.resume(cx))
}

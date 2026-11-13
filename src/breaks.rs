use std::time::{Duration, Instant};

use clap::ValueEnum;

use gpui_component::Sizable;
use gpui_kit::{
    Animation, AnimationExt, App, AsyncApp, Context, Global, Length, Pixels, Task, Window,
    WindowHandle, WindowOptions, component::button::Button, div, prelude::*, px, rgb,
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch::Receiver;

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

pub struct BreakRunner {
    window_handle: Option<WindowHandle<BreakView>>,
}
impl BreakRunner {
    pub fn new() -> Self {
        Self {
            window_handle: None,
        }
    }

    fn run_break(&mut self, cx: &mut App, break_type: BreakType) -> Task<()> {
        if self.window_handle.is_none() {
            let window_handle = cx
                .open_window(
                    WindowOptions {
                        ..Default::default()
                    },
                    |_, cx| cx.new(|cx| BreakView::new(cx, break_type)),
                )
                .unwrap();

            self.window_handle = Some(window_handle);
        }

        cx.spawn(async move |cx| {
            cx.background_executor().timer(break_type.duration()).await;
            cx.update_global::<BreakRunner, _>(|break_state, cx| {
                break_state
                    .window_handle
                    .take()
                    .map(|handle| handle.update(cx, |_, window, _| window.remove_window()));
            });
        })
    }
}

impl Global for BreakRunner {}

pub struct ActiveState {
    rx: Receiver<bool>,
}

impl ActiveState {
    pub fn new(rx: Receiver<bool>) -> Self {
        Self { rx }
    }

    pub fn running(&mut self) -> bool {
        *self.rx.borrow()
    }

    pub async fn wait_for_resume(&mut self) {
        while !*self.rx.borrow_and_update() {
            _ = self.rx.changed().await;
        }
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
        let break_text = div().child("Take a load off").text_2xl();
        let end_at = self.started_at + self.break_type.duration();
        let time_remaining = end_at.duration_since(Instant::now());
        let seconds_remaining = time_remaining.as_secs_f32();
        let minutes = (seconds_remaining.ceil() / 60.).round();
        let seconds = (seconds_remaining.ceil() % 60.).round();
        let time_remaining_text = format!("{minutes:02}:{seconds:02}");
        // let progress_bar_value = 1. - seconds_remaining / self.break_type.duration().as_secs_f32();
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
                    // .w(Length::Definite(gpui_kit::DefiniteLength::Fraction(
                    //     progress_bar_value,
                    // )))
                    .bg(rgb(0xFFFFFF))
                    .with_animation(
                        "break-progress-bar",
                        progress_bar_animation,
                        |progress_bar, progress_bar_value| {
                            progress_bar.w(Length::Definite(gpui_kit::DefiniteLength::Fraction(
                                progress_bar_value,
                            )))
                        },
                    ),
            );
        let skip_button = Button::new("skip-button")
            .px_6()
            .py_2()
            .text_center()
            .with_size(px(40.))
            .text_color(rgb(0xEDEDED))
            .border_2()
            .border_color(rgb(0xEDEDED))
            .bg(gpui_kit::transparent_black())
            .rounded(Pixels::from(9.))
            .label("Skip")
            .on_click(|_, window, _| window.remove_window());
        let center_div = div()
            .flex()
            .flex_col()
            .m_auto()
            .gap_5()
            .w(gpui_kit::Length::Definite(
                gpui_kit::DefiniteLength::Absolute(gpui_kit::AbsoluteLength::Rems(gpui_kit::Rems(
                    20.,
                ))),
            ))
            .h(gpui_kit::Length::Definite(
                gpui_kit::DefiniteLength::Absolute(gpui_kit::AbsoluteLength::Rems(gpui_kit::Rems(
                    20.,
                ))),
            ))
            .justify_between()
            .items_center()
            .child(break_text)
            .child(time_remaining_text)
            .child(progress_bar)
            .child(skip_button);
        div()
            .h_full()
            .w_full()
            .text_color(rgb(0xEDEDED))
            .bg(rgb(0x505050))
            .content_center()
            .child(center_div)
    }
}

pub fn start_break_scheduler(
    cx: &mut App,
    config: BreakConfig,
    mut active_state: ActiveState,
) -> Task<()> {
    cx.spawn(async move |cx| {
        let mut short_breaks_run = 0;
        loop {
            active_state.wait_for_resume().await;

            tracing::debug!("Scheduling next break");
            let (time_until_break, break_type) = config.next_break(short_breaks_run);
            cx.background_executor().timer(time_until_break).await;

            if !active_state.running() {
                tracing::debug!("System paused, waiting until resume");
                active_state.wait_for_resume().await;
                tracing::debug!("System resumed, continuing");
                // NOTE: Pausing and unpausing currently resets the break bacuse of `continue`
                //
                // If pausing and resuming should just pause the current break,
                // then the system needs to store the Instant when it was paused,
                // and add the difference to Instant::now() onto `run_break_at`.
                // `run_break_at` should be repeatedly checked against Instant::now(),
                // and run the break *IF* the system is running.
                // Maybe this can be a toggle in the config?
                continue;
            }
            short_breaks_run = (short_breaks_run + 1) % config.long_break_interval;
            run_break(cx, break_type).await;
        }
    })
}

pub async fn run_break(cx: &mut AsyncApp, break_type: BreakType) {
    tracing::debug!("Running break");
    cx.update_global::<BreakRunner, _>(|runner, cx| runner.run_break(cx, break_type))
        .await
}

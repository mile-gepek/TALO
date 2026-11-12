use std::time::{Duration, Instant};

use gpui_component::{Sizable, StyledExt};
use gpui_kit::{
    App, AsyncApp, Context, Global, Length, SharedString, Task, Window, WindowHandle, WindowId,
    WindowOptions, application, assets,
    component::{self, button::Button},
    div,
    prelude::*,
    px, rgb,
};

struct Daemon {
    _task: Task<()>,
}

impl Daemon {
    fn new(task: Task<()>) -> Self {
        Self { _task: task }
    }
}
impl Global for Daemon {}

struct Schedule {
    break_interval: Duration,
    long_break_interval: usize,
    short_breaks_run: usize,
    break_window: Option<WindowHandle<BreakView>>,
}

impl Schedule {
    pub fn new() -> Self {
        Self {
            break_interval: Duration::from_secs(5),
            long_break_interval: 2,
            short_breaks_run: 0,
            break_window: None,
        }
    }

    pub fn next_break(&mut self) -> (Duration, BreakType) {
        let break_type = if self.short_breaks_run >= self.long_break_interval {
            self.short_breaks_run = 0;
            BreakType::Long
        } else {
            self.short_breaks_run += 1;
            BreakType::Short
        };
        (self.break_interval, break_type)
    }
}

impl Global for Schedule {}

#[derive(Copy, Clone)]
enum BreakType {
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
    pub fn new(cx: &mut Context<Self>, break_type: BreakType, started_at: Instant) -> Self {
        let handle = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(20))
                    .await;
                this.update(cx, |_, cx| {
                    cx.notify();
                })
                .ok();
            }
        });
        BreakView {
            started_at,
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
        let progress_bar_value = 1. - seconds_remaining / self.break_type.duration().as_secs_f32();
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
                    .w(Length::Definite(gpui_kit::DefiniteLength::Fraction(
                        progress_bar_value,
                    )))
                    .bg(rgb(0xFFFFFF)),
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
            .rounded_full()
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

fn main() {
    application()
        .with_quit_mode(gpui_kit::QuitMode::Explicit)
        .with_assets(assets::Assets)
        .run(|cx: &mut App| {
            gpui_component::init(cx);
            cx.set_global(Schedule::new());

            let _schedule = cx.on_window_closed(|cx, id| {
                cx.update_global::<Schedule, _>(|s, _| {
                    if s.break_window
                        .is_some_and(|handle| handle.window_id() == id)
                    {
                        s.break_window = None;
                    }
                });
            });

            let daemon = cx.spawn(async move |cx| {
                let _schedule = _schedule;

                loop {
                    let (time_until_break, break_type) =
                        cx.update(|cx| cx.global_mut::<Schedule>().next_break());
                    cx.background_executor().timer(time_until_break).await;
                    let break_started_at = Instant::now();
                    let window_handle = run_break(cx, break_type, break_started_at);
                    cx.update_global::<Schedule, _>(|schedule, _| {
                        schedule.break_window = Some(window_handle);
                    });
                    while cx
                        .update_global::<Schedule, _>(|schedule, _| schedule.break_window.is_some())
                    {
                        if Instant::now() >= break_started_at + break_type.duration() {
                            cx.update_global::<Schedule, _>(|schedule, _| {
                                schedule.break_window = None
                            });
                            _ = window_handle.update(cx, |_, window, _| window.remove_window());
                        }
                        cx.background_executor()
                            .timer(Duration::from_millis(50))
                            .await;
                    }
                }
            });
            cx.set_global(Daemon::new(daemon))
        });
}

fn run_break(
    cx: &mut AsyncApp,
    break_type: BreakType,
    started_at: Instant,
) -> WindowHandle<BreakView> {
    cx.open_window(
        WindowOptions {
            ..Default::default()
        },
        |_, cx| cx.new(|cx| BreakView::new(cx, break_type, started_at)),
    )
    .unwrap()
}

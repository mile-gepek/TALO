use std::time::Duration;

use gpui_component::{
    ActiveTheme, Icon, IndexPath, Sizable, Size, StyledExt, h_flex,
    input::{Input, InputState},
    link::Link,
    scroll::ScrollableElement,
    searchable_list::SearchableListItem,
    select::Select,
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Div, Entity, Global, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, Styled, TitlebarOptions, WeakEntity, Window, WindowHandle,
    WindowOptions,
    base::{Transition, transition},
    div, px, rems,
};
use regex::Regex;

use crate::{
    components::InputField,
    config::{Config, ConfigService, load_config_or_default},
};
use crate::{components::SelectInput, config::AfterResumeBehavior};

type Row = AnyElement;
type Subsection = AnyElement;

impl SearchableListItem for AfterResumeBehavior {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::KeepScheduledBreak => "Keep Scheduled Break".into(),
            Self::Reschedule => "Reschedule".into(),
        }
    }

    fn value(&self) -> &Self::Value {
        self
    }
}

pub struct SettingsGlobal {
    window_handle: Option<WindowHandle<SettingsView>>,
}

impl Global for SettingsGlobal {}

impl SettingsGlobal {
    pub fn new() -> Self {
        Self {
            window_handle: None,
        }
    }
    pub(crate) fn open_or_focus(&mut self, cx: &mut App, config_weak: WeakEntity<ConfigService>) {
        let open_new = match self.window_handle {
            Some(handle) => handle
                .update(cx, |_, window, _| window.activate_window())
                .is_err(),
            _ => true,
        };
        if open_new {
            self.open(cx, config_weak)
        }
    }

    fn open(&mut self, cx: &mut App, config_service_weak: WeakEntity<ConfigService>) {
        let window_options = WindowOptions {
            app_id: Some("talo".to_string()),
            titlebar: Some(TitlebarOptions {
                title: Some("TALO - Settings".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let config = load_config_or_default();
        let window_handle = cx
            .open_window(window_options, |window, cx| {
                cx.new(|cx| SettingsView::new(window, cx, config, config_service_weak))
            })
            .unwrap();
        self.window_handle = Some(window_handle);
    }
}

fn min_after_resume_row(
    view: &SettingsView,
    enabled: bool,
    window: &mut Window,
    cx: &mut App,
) -> Row {
    let duration = cx.theme().motion_tokens().duration_fast;
    let easing = &cx.theme().motion_tokens().easing_move;

    let opacity_transition = transition(
        ("min-after-resume-row", "opacity"),
        if enabled { 1. } else { 0.5 },
        Transition::new(duration).easing(easing.clone()),
        window,
        cx,
    );

    let min_after_resume = row(
        label_with_hint(cx, "Minimum delay after resuming", "In seconds"),
        number_input(&view.min_after_resume.state(), enabled),
    );

    min_after_resume
        .opacity(opacity_transition)
        .into_any_element()
}

fn focus_guard(element: impl IntoElement) -> Div {
    div()
        .on_mouse_down_out(|_, window, cx| window.blur(cx))
        .child(element)
}

fn number_input(state: &Entity<InputState>, enabled: bool) -> Div {
    focus_guard(
        Input::new(state)
            .flex_none()
            .min_w(rems(4.))
            .text_align(gpui_kit::TextAlign::Center)
            .with_size(Size::Medium)
            .disabled(!enabled),
    )
}

fn rule(cx: &App) -> Div {
    let rule_color = cx.theme().border;
    div().h(px(1.)).w_full().bg(rule_color)
}

fn section(title: Option<&str>, subsections: Vec<Subsection>, cx: &App) -> Div {
    let padding = rems(1.25);

    let card_color = cx.theme().secondary;
    let corner_radius = cx.theme().radius * 2;
    let card = v_flex()
        .gap_8()
        .p(padding)
        .bg(card_color)
        .rounded(corner_radius)
        .children(subsections);

    let mut root = v_flex().gap_2();
    if let Some(title) = title {
        root = root.child(
            div()
                .child(title.to_string())
                .text_lg()
                .font_semibold()
                .ml(padding),
        );
    }
    root.child(card)
}

fn subsection(title: Option<&str>, options: Vec<Row>, cx: &App) -> Div {
    let header_text_color = cx.theme().muted_foreground;
    let mut root = v_flex();
    if let Some(title) = title {
        root = root.child(
            div()
                .child(title.to_string())
                .text_color(header_text_color)
                .text_base()
                .font_semibold()
                .mb_4(),
        );
    }
    root.child(
        v_flex()
            .children(
                options
                    .into_iter()
                    .flat_map(|option| [rule(cx).into_any_element(), option])
                    .skip(1),
            )
            .gap_4(),
    )
}

fn row(label: impl IntoElement, control: impl IntoElement) -> Div {
    let label = v_flex().items_start().flex_grow_1().child(label);
    h_flex()
        .justify_between()
        .items_center()
        .gap_3()
        .child(label)
        .child(control)
}

fn label_with_hint(cx: &App, label: impl IntoElement, hint: impl IntoElement) -> impl IntoElement {
    let hint_color = cx.theme().muted_foreground;
    v_flex()
        .child(label)
        .child(div().child(hint).text_sm().text_color(hint_color))
}

struct SettingsView {
    config_service_weak: WeakEntity<ConfigService>,
    short_break_duration: InputField<f32>,
    short_break_interval: InputField<f32>,
    long_break_duration: InputField<f32>,
    long_break_interval: InputField<usize>,
    after_resume: SelectInput<Vec<AfterResumeBehavior>>,
    min_after_resume: InputField<f32>,
}

impl SettingsView {
    fn update_config(&self, cx: &mut App, mutate: impl FnOnce(&mut Config)) {
        _ = self.config_service_weak.update(cx, |config_service, cx| {
            config_service.update_config(mutate, cx)
        });
    }

    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        initial_config: Config,
        config_service_weak: WeakEntity<ConfigService>,
    ) -> Self {
        let float_validator = |val: f32| val > 0.;

        let short_break_duration = InputField::new(
            initial_config.breaks.short_break_duration.as_secs_f32(),
            float_validator,
            window,
            cx,
            move |settings_view, seconds, _, cx| {
                settings_view.update_config(cx, |config| {
                    config.breaks.short_break_duration = Duration::from_secs_f32(*seconds)
                });
            },
        );
        let short_break_interval = InputField::new(
            initial_config.breaks.short_break_interval.as_secs_f32() / 60.,
            float_validator,
            window,
            cx,
            move |settings_view, minutes, _, cx| {
                settings_view.update_config(cx, |config| {
                    config.breaks.short_break_interval = Duration::from_secs_f32(minutes * 60.)
                });
            },
        );
        let long_break_duration = InputField::new(
            initial_config.breaks.long_break_duration.as_secs_f32() / 60.,
            float_validator,
            window,
            cx,
            move |settings_view, minutes, _, cx| {
                settings_view.update_config(cx, |config| {
                    config.breaks.long_break_duration = Duration::from_secs_f32(minutes * 60.)
                })
            },
        );
        let long_break_interval = InputField::new(
            initial_config.breaks.long_break_interval,
            |value| value > 0,
            window,
            cx,
            move |settings_view, count, _, cx| {
                settings_view.update_config(cx, |config| config.breaks.long_break_interval = *count)
            },
        );

        let after_resume = SelectInput::new(
            vec![
                AfterResumeBehavior::KeepScheduledBreak,
                AfterResumeBehavior::Reschedule,
            ],
            Some(IndexPath::new(0)),
            window,
            cx,
            move |settings_view, behavior, _, cx| {
                settings_view.update_config(cx, |config| {
                    let Some(behavior) = behavior else {
                        return;
                    };
                    config.breaks.after_resume_config.behavior = *behavior;
                })
            },
        );

        let min_after_resume = InputField::new(
            initial_config
                .breaks
                .after_resume_config
                .min_wait_after_resume
                .as_secs_f32(),
            float_validator,
            window,
            cx,
            move |settings_view, seconds, _, cx| {
                settings_view.update_config(cx, |config| {
                    config.breaks.after_resume_config.min_wait_after_resume =
                        Duration::from_secs_f32(*seconds)
                })
            },
        );

        Self {
            short_break_duration,
            short_break_interval,
            long_break_duration,
            long_break_interval,
            after_resume,
            min_after_resume,
            config_service_weak: config_service_weak,
        }
    }

    fn about_section(&self, cx: &App) -> AnyElement {
        let github_repo = Link::new("github-link")
            .child(
                Icon::empty()
                    .m_3()
                    .path("icons/github.svg")
                    .text_color(cx.theme().muted_foreground),
            )
            .on_click(|_, _, cx| {
                cx.open_url("https://github.com/mile-gepek/talo");
            });
        let version = row("Version 0.1.0", github_repo);

        section(Some("About"), vec![version.into_any_element()], cx).into_any_element()
    }

    fn breaks_section(
        &self,
        keep_time_remaining: bool,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let general = subsection(
            Some("General"),
            vec![
                row(
                    label_with_hint(cx, "Short break duration", "In seconds"),
                    number_input(self.short_break_duration.state(), true),
                )
                .into_any_element(),
                row(
                    label_with_hint(cx, "Interval between short breaks", "In minutes"),
                    number_input(self.short_break_interval.state(), true),
                )
                .into_any_element(),
                row(
                    label_with_hint(cx, "Long break duration", "In seconds"),
                    number_input(self.long_break_duration.state(), true),
                )
                .into_any_element(),
                row(
                    label_with_hint(
                        cx,
                        "Interval between long breaks",
                        "Number of short breaks before a long break",
                    ),
                    number_input(self.long_break_interval.state(), true),
                )
                .into_any_element(),
            ],
            cx,
        );

        let after_resuming = subsection(
            Some("After resuming"),
            vec![
                row(
                    "Behavior after resuming",
                    div()
                        .w(rems(12.))
                        .child(Select::new(self.after_resume.state()).with_size(Size::Medium)),
                )
                .into_any_element(),
                min_after_resume_row(self, keep_time_remaining, window, cx),
            ],
            cx,
        );

        section(
            Some("Breaks"),
            vec![
                general.into_any_element(),
                after_resuming.into_any_element(),
            ],
            cx,
        )
        .into_any_element()
    }
}

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let keep_time_remaining =
            self.after_resume.state().read(cx).selected_index(cx) == Some(IndexPath::new(0));

        v_flex()
            .size_full()
            .overflow_y_scrollbar()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .w(rems(50.))
                    .mx_auto()
                    .py_8()
                    .gap_6()
                    .child(self.about_section(cx))
                    .child(self.breaks_section(keep_time_remaining, window, cx)),
            )
            .font_family("Liberation Sans")
    }
}

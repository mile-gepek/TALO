use gpui_component::{
    ActiveTheme, Disableable, Icon, IndexPath, StyledExt, h_flex,
    input::{InputState, NumberInput},
    scroll::ScrollableElement,
    searchable_list::SearchableListItem,
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use gpui_kit::{
    AnyElement, App, AppContext, Context, Div, Entity, Global, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, TitlebarOptions, Window, WindowHandle, WindowOptions,
    base::{Transition, transition},
    div, px, rems,
};

use crate::config::AfterResumeBehavior;

type Row = AnyElement;
type Subsection = AnyElement;

impl SearchableListItem for AfterResumeBehavior {
    type Value = Self;

    fn title(&self) -> SharedString {
        match self {
            Self::KeepTimeRemaining => "Keep Scheduled Break".into(),
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
    pub(crate) fn open_or_focus(&mut self, cx: &mut App) {
        let open_new = match self.window_handle {
            Some(handle) => handle
                .update(cx, |_, window, _| window.activate_window())
                .is_err(),
            _ => true,
        };
        if open_new {
            self.open(cx)
        }
    }

    fn open(&mut self, cx: &mut App) {
        let window_options = WindowOptions {
            app_id: Some("talo".to_string()),
            titlebar: Some(TitlebarOptions {
                title: Some("TALO - Settings".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let window_handle = cx
            .open_window(window_options, |window, cx| {
                cx.new(|cx| SettingsView::new(window, cx))
            })
            .unwrap();
        self.window_handle = Some(window_handle);
    }
}

struct SettingsView {
    short_break_duration: Entity<InputState>,
    short_break_interval: Entity<InputState>,
    long_break_duration: Entity<InputState>,
    long_break_after: Entity<InputState>,
    after_resume: Entity<SelectState<Vec<AfterResumeBehavior>>>,
    min_after_resume: Entity<InputState>,
    _after_resume_subscription: Subscription,
}

impl SettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let after_resume = cx.new(|cx| {
            SelectState::new(
                vec![
                    AfterResumeBehavior::KeepTimeRemaining,
                    AfterResumeBehavior::Reschedule,
                ],
                Some(IndexPath::new(0)),
                window,
                cx,
            )
        });
        let _after_resume_subscription = cx.subscribe(
            &after_resume,
            |_, _, _: &SelectEvent<Vec<AfterResumeBehavior>>, cx| cx.notify(),
        );

        Self {
            short_break_duration: number_input_state(window, cx, 1.),
            short_break_interval: number_input_state(window, cx, 1.),
            long_break_duration: number_input_state(window, cx, 1.),
            long_break_after: number_input_state(window, cx, 1.),
            after_resume,
            min_after_resume: number_input_state(window, cx, 1.),
            _after_resume_subscription,
        }
    }

    fn about_section(&self, cx: &App) -> AnyElement {
        let version = row(
            "Version 0.1.0",
            Icon::empty()
                .path("icons/github.svg")
                .text_color(cx.theme().muted_foreground),
        );

        section(Some("About"), vec![version.into_any_element()], cx).into_any_element()
    }

    fn breaks_section(
        &self,
        keep_time_remaining: bool,
        window: &mut Window,
        mut cx: &mut App,
    ) -> AnyElement {
        let general = subsection(
            Some("General"),
            vec![
                row(
                    label_with_hint(cx, "Short break duration", "In seconds"),
                    number_input(&self.short_break_duration, true),
                )
                .into_any_element(),
                row(
                    label_with_hint(cx, "Interval between short breaks", "In minutes"),
                    number_input(&self.short_break_interval, true),
                )
                .into_any_element(),
                row(
                    label_with_hint(cx, "Long break duration", "In seconds"),
                    number_input(&self.long_break_duration, true),
                )
                .into_any_element(),
                row(
                    label_with_hint(
                        cx,
                        "Interval between long breaks",
                        "Number of short breaks before a long break",
                    ),
                    number_input(&self.long_break_after, true),
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
                    div().w(rems(14.)).child(Select::new(&self.after_resume)),
                )
                .into_any_element(),
                min_after_resume_row(self, keep_time_remaining, window, &mut cx),
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

fn min_after_resume_row(
    view: &SettingsView,
    enabled: bool,
    window: &mut Window,
    mut cx: &mut App,
) -> Row {
    let duration = cx.theme().motion_tokens().duration_fast;
    let easing = &cx.theme().motion_tokens().easing_move;

    let opacity_transition = transition(
        ("min-after-resume-row", "opacity"),
        if enabled { 1. } else { 0.5 },
        Transition::new(duration).easing(easing.clone()),
        window,
        &mut cx,
    );

    let min_after_resume = row(
        label_with_hint(cx, "Minimum delay after resuming", "In seconds"),
        number_input(&view.min_after_resume, enabled),
    );

    min_after_resume
        .opacity(opacity_transition)
        .into_any_element()
}

fn number_input_state(
    window: &mut Window,
    cx: &mut Context<SettingsView>,
    step: f64,
) -> Entity<InputState> {
    cx.new(|cx| InputState::new(window, cx).placeholder("0").step(step))
}

fn number_input(state: &Entity<InputState>, enabled: bool) -> NumberInput {
    NumberInput::new(state)
        .flex_none()
        .min_w(rems(7.))
        .disabled(!enabled)
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
    let mut root = v_flex();
    if let Some(title) = title {
        root = root.child(
            div()
                .child(title.to_string())
                // .text_color(header_text_color)
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

impl Render for SettingsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let keep_time_remaining =
            self.after_resume.read(cx).selected_index(cx) == Some(IndexPath::new(0));

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

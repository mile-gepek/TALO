use std::{fmt::Display, marker::PhantomData, str::FromStr};

use gpui_component::input::{InputEvent, InputState};
use gpui_kit::{AppContext, Context, Entity, Subscription, Window};

pub struct InputField<T> {
    input_state: Entity<InputState>,
    _subscription: Subscription,
    _phantom: PhantomData<T>,
}

impl<T: Display + FromStr + 'static> InputField<T> {
    pub fn new<V: 'static>(
        initial_value: T,
        window: &mut Window,
        cx: &mut Context<V>,
        on_change: impl Fn(&mut V, &T, Entity<InputState>, &mut Context<V>) + 'static,
    ) -> Self
    where
        <T as FromStr>::Err: std::fmt::Debug,
    {
        let state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("0")
                .step(1.)
                .default_value(initial_value.to_string())
        });
        let subscription = cx.subscribe(&state, move |this, state, event, cx| {
            let value = state.read(cx).value().parse().unwrap();
            match event {
                InputEvent::Change | InputEvent::PressEnter { .. } => {
                    on_change(this, &value, state, cx);
                }
                _ => {}
            }
        });
        Self {
            input_state: state,
            _subscription: subscription,
            _phantom: PhantomData,
        }
    }

    pub fn state(&self) -> &Entity<InputState> {
        &self.input_state
    }
}

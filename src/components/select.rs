use gpui_component::{
    IndexPath,
    searchable_list::{SearchableListDelegate, SearchableListItem},
    select::{SelectEvent, SelectState},
};
use gpui_kit::{AppContext, Context, Entity, Subscription, Window};

pub struct SelectInput<D: SearchableListDelegate> {
    state: Entity<SelectState<D>>,
    _subscription: Subscription,
}

impl<D: SearchableListDelegate> SelectInput<D> {
    pub fn new<V: 'static>(
        items: D,
        selected_index: Option<IndexPath>,
        window: &mut Window,
        cx: &mut Context<V>,
        on_change: fn(
            &mut V,
            &Option<<D::Item as SearchableListItem>::Value>,
            Entity<SelectState<D>>,
            &mut Context<V>,
        ),
    ) -> Self {
        let state = cx.new(|cx| SelectState::new(items, selected_index, window, cx));

        let subscription =
            cx.subscribe(
                &state,
                move |this, state, event: &SelectEvent<D>, cx| match event {
                    SelectEvent::Confirm(value) => on_change(this, value, state, cx),
                },
            );

        Self {
            state,
            _subscription: subscription,
        }
    }

    pub fn state(&self) -> &Entity<SelectState<D>> {
        &self.state
    }
}

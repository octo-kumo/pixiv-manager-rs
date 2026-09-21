//! A [`TextInput`] wrapper that reports when it gains/loses focus.
//!
//! iced 0.14 exposes no global focus events, so the tag-suggestion dropdown
//! (which should appear only while the "Add tags" field is focused) can't
//! know whether to show itself from the app layer alone. This wrapper embeds
//! the real `TextInput` as a child, forwards every event to it, and — after
//! the child processes an event — compares its internal `is_focused` state to
//! the previously observed one. On a transition it publishes
//! [`Message::TagFocusChanged`], which the update loop mirrors into app state
//! so the dropdown visibility tracks focus exactly.

use iced::advanced::layout;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tag, Tree};
use iced::advanced::widget::{Operation, Widget};
use iced::advanced::{mouse, Clipboard, Layout, Shell};
use iced::{Event, Length, Rectangle, Size, Vector};

/// The wrapper's own tree state: the last known focus of the child input.
#[derive(Debug, Default)]
struct FocusState {
    focused: bool,
}

/// A focus-reporting wrapper around a [`iced::widget::TextInput`].
pub struct FocusTrackTextInput<'a, Message> {
    /// The wrapped input, built fresh by the caller each frame (its tree
    /// state — focus, cursor, drag — survives the rebuild via the diff).
    input: iced::Element<'a, Message, iced::Theme, iced::Renderer>,
    /// Called with the new focus value when the child's focus changes.
    on_focus_changed: Option<fn(bool) -> Message>,
}

impl<'a, Message> FocusTrackTextInput<'a, Message>
where
    Message: Clone + 'a,
{
    /// Wrap a [`iced::widget::TextInput`] element (usually the result of
    /// `TextInput::new(...).on_input(...).on_submit(...)`).
    pub fn new(input: iced::widget::TextInput<'a, Message>) -> Self {
        Self {
            input: input.into(),
            on_focus_changed: None,
        }
    }

    /// Set the message builder invoked on a focus gain/loss transition.
    pub fn on_focus_changed(mut self, f: fn(bool) -> Message) -> Self {
        self.on_focus_changed = Some(f);
        self
    }

    fn child_state(
        tree: &Tree,
    ) -> &iced::widget::text_input::State<
        <iced::Renderer as iced::advanced::text::Renderer>::Paragraph,
    > {
        tree.children[0]
            .state
            .downcast_ref::<iced::widget::text_input::State<
                <iced::Renderer as iced::advanced::text::Renderer>::Paragraph,
            >>()
    }
}

impl<'a, Message> Widget<Message, iced::Theme, iced::Renderer> for FocusTrackTextInput<'a, Message>
where
    Message: 'a + Clone,
{
    fn tag(&self) -> Tag {
        Tag::of::<FocusState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(FocusState::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.input)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.input]);
    }

    fn size(&self) -> Size<Length> {
        self.input.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.input
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.input
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        self.input.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );
        // Detect a focus transition and publish it so the app can show/hide
        // the tag-suggestion dropdown.
        let focused = Self::child_state(tree).is_focused();
        let state = tree.state.downcast_mut::<FocusState>();
        if state.focused != focused {
            state.focused = focused;
            if let Some(f) = self.on_focus_changed {
                shell.publish(f(focused));
            }
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.input.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.input.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<iced::advanced::overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.input.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message> From<FocusTrackTextInput<'a, Message>>
    for iced::Element<'a, Message, iced::Theme, iced::Renderer>
where
    Message: 'a + Clone,
{
    fn from(widget: FocusTrackTextInput<'a, Message>) -> Self {
        Self::new(widget)
    }
}

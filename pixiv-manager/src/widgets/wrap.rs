//! A widget that lays out its children left-to-right, wrapping onto new lines
//! when they exceed the available width. Used for tag lists, where a plain
//! `Row` would push the last item onto its own line instead of wrapping.

use iced::advanced::layout::{self, Layout, Node};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::tree::{self, Tag, Tree};
use iced::advanced::widget::{Operation, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::mouse;
use iced::{Element, Event, Length, Rectangle, Size, Vector};

/// Marker state for the wrap widget (children carry their own state).
#[derive(Default)]
struct WrapState;

/// A container that wraps its children into multiple rows.
pub struct Wrap<'a, Message, Theme = iced::Theme, Renderer = iced::Renderer> {
    children: Vec<Element<'a, Message, Theme, Renderer>>,
    spacing: f32,
    width: Length,
}

impl<'a, Message, Theme, Renderer> Wrap<'a, Message, Theme, Renderer> {
    /// Create a wrap container with the given children.
    pub fn with_children(children: Vec<Element<'a, Message, Theme, Renderer>>) -> Self {
        Self {
            children,
            spacing: 4.0,
            width: Length::Fill,
        }
    }

    /// Set the gap between children.
    pub fn spacing(mut self, spacing: f32) -> Self {
        self.spacing = spacing;
        self
    }

    /// Set the container width.
    pub fn width(mut self, width: impl Into<Length>) -> Self {
        self.width = width.into();
        self
    }
}

impl<'a, Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Wrap<'a, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
    Message: Clone,
{
    fn tag(&self) -> Tag {
        Tag::of::<WrapState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(WrapState)
    }

    fn children(&self) -> Vec<Tree> {
        self.children.iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.children);
    }

    fn size(&self) -> Size<Length> {
        Size {
            width: self.width,
            height: Length::Shrink,
        }
    }

    fn layout(&mut self, tree: &mut Tree, renderer: &Renderer, limits: &layout::Limits) -> Node {
        let limits = limits.width(self.width);
        let max_width = limits.max().width;
        let spacing = self.spacing;

        let child_limits = limits.loose();
        let mut children: Vec<Node> = Vec::with_capacity(self.children.len());
        let mut intrinsic_width = 0.0f32;
        let mut y = 0.0f32;
        let mut x = 0.0f32;
        let mut row_height = 0.0f32;

        for (i, child) in self.children.iter_mut().enumerate() {
            let node = child
                .as_widget_mut()
                .layout(&mut tree.children[i], renderer, &child_limits);
            let child_size = node.size();

            if x != 0.0 && x + child_size.width > max_width {
                intrinsic_width = intrinsic_width.max(x - spacing);
                y += row_height + spacing;
                x = 0.0;
                row_height = 0.0;
            }

            row_height = row_height.max(child_size.height);
            children.push(node.move_to((x, y)));
            x += child_size.width + spacing;
        }

        if x != 0.0 {
            intrinsic_width = intrinsic_width.max(x - spacing);
        }
        let intrinsic_height = y + row_height;

        Node::with_children(Size::new(intrinsic_width, intrinsic_height), children)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        operation.container(None, layout.bounds());
        for (i, child) in self.children.iter_mut().enumerate() {
            child.as_widget_mut().operate(
                &mut tree.children[i],
                layout.children().nth(i).unwrap(),
                renderer,
                operation,
            );
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        for (i, child) in self.children.iter_mut().enumerate() {
            child.as_widget_mut().update(
                &mut tree.children[i],
                event,
                layout.children().nth(i).unwrap(),
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        for (i, child) in self.children.iter().enumerate() {
            child.as_widget().draw(
                &tree.children[i],
                renderer,
                theme,
                style,
                layout.children().nth(i).unwrap(),
                cursor,
                viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.children
            .iter()
            .zip(tree.children.iter())
            .zip(layout.children())
            .map(|((child, tree), layout)| {
                child
                    .as_widget()
                    .mouse_interaction(tree, layout, cursor, viewport, renderer)
            })
            .max()
            .unwrap_or(mouse::Interaction::default())
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        overlay::from_children(
            &mut self.children,
            tree,
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Wrap<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a + Clone,
    Theme: 'a,
    Renderer: 'a + renderer::Renderer,
{
    fn from(wrap: Wrap<'a, Message, Theme, Renderer>) -> Self {
        Element::new(wrap)
    }
}

/// Helper to build a wrapping container.
pub fn wrap<'a, Message, Theme, Renderer>(
    children: Vec<Element<'a, Message, Theme, Renderer>>,
) -> Wrap<'a, Message, Theme, Renderer> {
    Wrap::with_children(children)
}

//! The iced inspector overlay (§13 "Overlay: adapter-level composition —
//! iced: `Operation`-returned rects").
//!
//! The plan is *data* ([`OverlayPlan`]); rendering it never triggers an
//! extra pass — the wrapper program adds one overlay widget to the app's
//! own view tree, so the Emulator's existing UI build (and therefore the
//! headless screenshot pass) paints the hover outline + role badges
//! through iced's real draw pipeline. No framework-free code runs here:
//! the whole module is feature-gated (§18).
//!
//! The overlay widget records every draw into a shared paint log — the
//! §13 draw-call evidence the pixel-diff tests assert against (M83).

use std::cell::RefCell;
use std::rc::Rc;

use guiwright_inspect::overlay::OverlayPlan;
use iced_test::core::{
    Color, Element, Length, Pixels, Rectangle, Size, alignment,
    layout::{self, Layout, Limits},
    mouse,
    renderer::{Quad, Style},
    text::{LineHeight, Shaping, Text, Wrapping},
    widget::{self, Tree},
};
use iced_test::program::Program;
use iced_test::runtime::Task;
use iced_widget::Stack;

/// Outline color (the kittest-ish debug blue the egui overlay paints).
const OUTLINE: Color = Color::from_rgb(
    0x31 as f32 / 255.0,
    0x61 as f32 / 255.0,
    0x9f as f32 / 255.0,
);
/// Badge background (dim ink, opaque — the headless raster has no blend
/// target below it beyond the app's own scene).
const BADGE_BG: Color = Color::from_rgb(
    0x1c as f32 / 255.0,
    0x1f as f32 / 255.0,
    0x24 as f32 / 255.0,
);
/// Badge text.
const BADGE_FG: Color = Color::from_rgb(
    0xdf as f32 / 255.0,
    0xe3 as f32 / 255.0,
    0xe8 as f32 / 255.0,
);
/// Badge chip metrics (logical px): a debug overlay needs no font
/// measurement — the chip rides the draw log, the pixel diff proves the
/// paint.
const BADGE_FONT_SIZE: f32 = 11.0;
const BADGE_CHAR_W: f32 = 6.5;
const BADGE_HEIGHT: f32 = 16.0;

/// The shared overlay state: the plan the wrapper program composites and
/// the draw calls the widget performed (the §13 draw-call evidence).
#[derive(Debug, Default)]
pub struct OverlayInner {
    plan: Option<OverlayPlan>,
    paint_log: Vec<String>,
}

/// The overlay state handle shared between the harness (which sets the
/// plan) and the overlay widget (which paints it).
#[derive(Clone, Debug, Default)]
pub struct OverlayState(Rc<RefCell<OverlayInner>>);

impl OverlayState {
    /// Install the plan for the next view build (`None` stops
    /// compositing — the overlay session ended).
    pub fn set_plan(&self, plan: Option<&OverlayPlan>) {
        let mut inner = self.0.borrow_mut();
        inner.plan = plan.cloned();
    }

    /// Drain the paint log (one entry per draw call, oldest first).
    pub fn take_paint_log(&self) -> Vec<String> {
        std::mem::take(&mut self.0.borrow_mut().paint_log)
    }

    fn push_log(&self, entry: String) {
        self.0.borrow_mut().paint_log.push(entry);
    }

    fn plan(&self) -> Option<OverlayPlan> {
        self.0.borrow().plan.clone()
    }
}

/// The overlay layer widget: paints the plan inside the app's own pass —
/// hover outline + badge chips over the laid-out scene, zero widget-tree
/// impact (a leaf in a [`Stack`], no operations, no event handling).
#[derive(Debug)]
pub struct OverlayLayer {
    state: OverlayState,
}

impl OverlayLayer {}

impl<Message, Theme, Renderer> widget::Widget<Message, Theme, Renderer> for OverlayLayer
where
    Renderer: iced_test::core::Renderer + iced_test::core::text::Renderer,
    Renderer::Font: Default,
{
    /// The overlay fills the window: a [`Stack`] child overlays the app's
    /// view exactly (§13 "hover outlines + role badges onto the app's
    /// scene").
    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(&mut self, _tree: &mut Tree, _renderer: &Renderer, limits: &Limits) -> layout::Node {
        layout::Node::new(limits.resolve(Length::Fill, Length::Fill, Size::ZERO))
    }

    fn draw(
        &self,
        _tree: &Tree,
        renderer: &mut Renderer,
        _theme: &Theme,
        _style: &Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        let Some(plan) = self.state.plan() else {
            return;
        };
        if plan.is_empty() {
            return;
        }

        // The hover outline: translucent fill + 1.5px border (the egui
        // overlay's shape, painted through iced's quad pipeline).
        if let Some(rect) = &plan.hover {
            let bounds = Rectangle {
                x: rect.x0 as f32,
                y: rect.y0 as f32,
                width: (rect.x1 - rect.x0) as f32,
                height: (rect.y1 - rect.y0) as f32,
            };
            renderer.with_layer(bounds, |renderer| {
                renderer.fill_quad(
                    Quad {
                        bounds,
                        border: iced_test::core::Border {
                            radius: 2.0.into(),
                            width: 1.5,
                            color: OUTLINE,
                        },
                        shadow: iced_test::core::Shadow::default(),
                        snap: false,
                    },
                    Color::from_rgba(
                        0x4f as f32 / 255.0,
                        0x9c as f32 / 255.0,
                        0xf9 as f32 / 255.0,
                        0.09,
                    ),
                );
            });
            self.state.push_log(format!(
                "outline {:.0},{:.0}→{:.0},{:.0}",
                rect.x0, rect.y0, rect.x1, rect.y1
            ));
        }

        // Badge chips: one fill_quad (chip) + one fill_text (caption)
        // per badge, pinned at the node's laid-out center.
        for badge in &plan.badges {
            let width = BADGE_CHAR_W * badge.text.chars().count() as f32 + 8.0;
            let chip = Rectangle {
                x: (badge.at.0 - f64::from(width) / 2.0) as f32,
                y: (badge.at.1 - f64::from(BADGE_HEIGHT) / 2.0) as f32,
                width,
                height: BADGE_HEIGHT,
            };
            renderer.with_layer(chip, |renderer| {
                renderer.fill_quad(
                    Quad {
                        bounds: chip,
                        border: iced_test::core::Border {
                            radius: 3.0.into(),
                            width: 1.0,
                            color: OUTLINE,
                        },
                        shadow: iced_test::core::Shadow::default(),
                        snap: false,
                    },
                    BADGE_BG,
                );
                renderer.fill_text(
                    Text {
                        content: badge.text.clone(),
                        bounds: chip.size(),
                        size: Pixels::from(BADGE_FONT_SIZE),
                        line_height: LineHeight::default(),
                        font: <Renderer as iced_test::core::text::Renderer>::Font::default(),
                        align_x: iced_test::core::text::Alignment::Center,
                        align_y: alignment::Vertical::Center,
                        shaping: Shaping::Basic,
                        wrapping: Wrapping::None,
                    },
                    chip.center(),
                    BADGE_FG,
                    *viewport,
                );
            });
            self.state.push_log(format!(
                "badge {:?} @ {:.0},{:.0}",
                badge.text, badge.at.0, badge.at.1
            ));
        }
    }
}

/// The wrapper program: the app's own [`Program`] with the overlay layer
/// composited on top when a plan is active (§13 "adapter composites at
/// present time" — the composition happens in the app's own view build,
/// so every Emulator pass carries it).
#[derive(Debug)]
pub struct OverlayProgram<P: Program> {
    inner: P,
    state: OverlayState,
}

impl<P: Program> OverlayProgram<P> {
    /// Wrap `program`: the overlay reads `state` (the shared
    /// [`OverlayState`]) at every view build.
    pub fn new(inner: P, state: OverlayState) -> Self {
        Self { inner, state }
    }

    /// The shared overlay state (the harness's plan + paint-log handle).
    pub fn overlay_state(&self) -> &OverlayState {
        &self.state
    }
}

impl<P: Program> Program for OverlayProgram<P>
where
    // The overlay layer paints badge captions through the renderer's text
    // pipeline — the harness's `tiny-skia` renderer satisfies this (the
    // §11 headless row); a null renderer would silently skip the layer.
    // The theme/renderer ride 'static like every concrete iced app's
    // (the Element boxes are `+ 'a` for any 'a).
    P::Renderer: iced_test::core::text::Renderer,
    <P::Renderer as iced_test::core::text::Renderer>::Font: Default,
    P::Theme: 'static,
    P::Renderer: 'static,
{
    type State = P::State;
    type Message = P::Message;
    type Theme = P::Theme;
    type Renderer = P::Renderer;
    type Executor = P::Executor;

    fn name() -> &'static str {
        P::name()
    }

    fn settings(&self) -> iced_test::core::Settings {
        self.inner.settings()
    }

    fn window(&self) -> Option<iced_test::core::window::Settings> {
        self.inner.window()
    }

    fn boot(&self) -> (Self::State, Task<Self::Message>) {
        self.inner.boot()
    }

    fn update(&self, state: &mut Self::State, message: Self::Message) -> Task<Self::Message> {
        self.inner.update(state, message)
    }

    fn view<'a>(
        &self,
        state: &'a Self::State,
        window: iced_test::core::window::Id,
    ) -> Element<'a, Self::Message, Self::Theme, Self::Renderer> {
        let inner = self.inner.view(state, window);
        if self.state.plan().is_none() {
            return inner;
        }
        Stack::with_children(vec![
            inner,
            Element::new(OverlayLayer {
                state: self.state.clone(),
            }),
        ])
        .into()
    }

    fn subscription(&self, state: &Self::State) -> iced_test::futures::Subscription<Self::Message> {
        self.inner.subscription(state)
    }

    fn theme(
        &self,
        state: &Self::State,
        window: iced_test::core::window::Id,
    ) -> Option<Self::Theme> {
        self.inner.theme(state, window)
    }

    fn style(&self, state: &Self::State, theme: &Self::Theme) -> iced_test::core::theme::Style {
        self.inner.style(state, theme)
    }

    fn title(&self, state: &Self::State, window: iced_test::core::window::Id) -> String {
        self.inner.title(state, window)
    }

    fn scale_factor(&self, state: &Self::State, window: iced_test::core::window::Id) -> f32 {
        self.inner.scale_factor(state, window)
    }
}

//! Embedded egui adapter integration tests (architecture §4.1, §6.3).
//!
//! These drive a real `egui::Context` through the adapter — a windowless
//! Level-1 path with AccessKit tree generation, semantic actions, synthetic
//! input and the virtual clock. They run through the standard
//! [`guiwright_driver::Session`] so embedded and remote behave identically.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use guiwright_adapter_egui::EguiAdapter;
use guiwright_core::Selector;
use guiwright_driver::FrameworkAdapter;
use guiwright_driver::adapter::{InputEvent, PointerButton, SemAction};
use guiwright_driver::session::{ActionOpts, Session};

/// The canonical demo app: counter buttons and a text input, all stock egui
/// widgets.
struct CounterApp {
    clicks: usize,
    email: String,
    greeting: String,
}

impl CounterApp {
    fn ui(&mut self, ctx: &egui::Context) {
        // egui 0.37: panels are shown against the root Ui.
        ctx.root_ui(|ui| {
            egui::CentralPanel::default_margins().show(ui, |ui| {
                ui.label(format!("Welcome, {}", self.greeting));
                if ui.button("Save").clicked() {
                    self.clicks += 1;
                }
                if ui.button("Cancel").clicked() {
                    self.clicks = 0;
                }
                ui.add(egui::TextEdit::singleline(&mut self.email).id_salt("email"));
            });
        });
    }
}

fn counter_session() -> (Session<EguiAdapter>, Rc<RefCell<CounterApp>>) {
    let app_state = Rc::new(RefCell::new(CounterApp {
        clicks: 0,
        email: String::new(),
        greeting: "Ada".into(),
    }));
    let state = Rc::clone(&app_state);
    let adapter = EguiAdapter::new(move |ctx: &egui::Context| {
        let mut app = state.borrow_mut();
        app.ui(ctx);
    });
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    (session, app_state)
}

fn save() -> Selector {
    Selector::role_named(accesskit::Role::Button, "Save")
}

fn email_input() -> Selector {
    Selector::role(accesskit::Role::TextInput)
}

#[test]
fn accesskit_tree_is_generated_with_bounds() {
    let (session, _) = counter_session();
    let tree = session.tree();
    // Save button resolves with bounds (Level-1 tree with geometry).
    let resolved = tree
        .resolve(&save())
        .expect("egui emits an AccessKit tree with the button");
    let bounds = resolved.bounds.expect("button has bounds");
    assert!(bounds.area() > 0.0, "bounds: {bounds:?}");

    // Label text resolves through getByText — egui emits both a Label node
    // and its TextRun child with the same text, so strict mode requires
    // narrowing (identical to Playwright's strict-mode behavior).
    tree.resolve(&Selector::text("Welcome, Ada").first())
        .expect("label text");
    // TextEdit resolves as a TextInput role.
    tree.resolve(&email_input()).expect("text edit in the tree");
}

#[test]
fn semantic_click_increments_counter() {
    let (mut session, state) = counter_session();
    session.click(&save()).expect("click Save");
    assert_eq!(
        state.borrow().clicks,
        1,
        "AccessKit Click drives the button"
    );

    // Playwright-style repeat: click again through the same locator.
    session.click(&save()).expect("click again");
    assert_eq!(state.borrow().clicks, 2);

    // Cancel resets.
    session
        .click(&Selector::role_named(accesskit::Role::Button, "Cancel"))
        .expect("click Cancel");
    assert_eq!(state.borrow().clicks, 0);
}

#[test]
fn fill_sets_value_on_text_edit() {
    let (mut session, state) = counter_session();
    session
        .fill(&email_input(), "ada@example.dev")
        .expect("fill email");
    assert_eq!(state.borrow().email, "ada@example.dev");
}

#[test]
fn assertions_react_to_state_changes() {
    let (mut session, _state) = counter_session();
    session
        .expect(Selector::text("Welcome, Ada").first())
        .to_be_visible()
        .expect("greeting visible");
    session
        .expect(save())
        .to_be_enabled()
        .expect("save enabled");
}

#[test]
fn synthetic_pointer_click_works_too() {
    let (mut session, state) = counter_session();
    // Resolve → center coordinates → real pointer pipeline (§9).
    let resolved = session.resolve(&save()).expect("resolve save");
    let rect = resolved.bounds.expect("bounds");
    let cx = f64::midpoint(rect.x0, rect.x1);
    let cy = f64::midpoint(rect.y0, rect.y1);

    session
        .adapter_mut()
        .input(InputEvent::PointerMove { x: cx, y: cy })
        .unwrap();
    session
        .adapter_mut()
        .input(InputEvent::PointerDown {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .unwrap();
    session
        .adapter_mut()
        .input(InputEvent::PointerUp {
            x: cx,
            y: cy,
            button: PointerButton::Primary,
        })
        .unwrap();
    session.pump();
    assert_eq!(state.borrow().clicks, 1, "synthetic click == OS click");
}

#[test]
fn virtual_clock_advances_egui_time() {
    let (mut session, _) = counter_session();
    session
        .clock_fast_forward(Duration::from_secs(30))
        .expect("egui clock is RawInput.time (§14)");
    assert!(
        session.adapter().virtual_time() >= Duration::from_secs(29),
        "virtual time moved"
    );
}

#[test]
fn hover_rides_the_real_input_pipeline() {
    let (mut session, _state) = counter_session();
    session
        .perform_action(&save(), SemAction::Hover, &ActionOpts::default())
        .expect("hover via pointer move");
}

#[test]
fn strict_mode_catches_two_save_buttons() {
    let app_state = Rc::new(RefCell::new(CounterApp {
        clicks: 0,
        email: String::new(),
        greeting: "Ada".into(),
    }));
    let state = Rc::clone(&app_state);
    let adapter = EguiAdapter::new(move |ctx: &egui::Context| {
        let _app = state.borrow_mut();
        ctx.root_ui(|ui| {
            egui::CentralPanel::default_margins().show(ui, |ui| {
                let _ = ui.button("Save");
                let _ = ui.button("Save");
            });
        });
    });
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));
    let err = session.click(&save()).expect_err("two buttons");
    assert!(err.to_string().contains("resolved to 2"), "{err}");
}

#[test]
fn dynamic_ui_update_is_observed_by_waiters() {
    // Greeting appears only after a click — auto-wait handles it (§8.3).
    let app_state = Rc::new(RefCell::new(CounterApp {
        clicks: 0,
        email: String::new(),
        greeting: String::new(),
    }));
    let state = Rc::clone(&app_state);
    let adapter = EguiAdapter::new(move |ctx: &egui::Context| {
        let mut app = state.borrow_mut();
        ctx.root_ui(|ui| {
            egui::CentralPanel::default_margins().show(ui, |ui| {
                if !app.greeting.is_empty() {
                    ui.label(format!("Welcome, {}", app.greeting));
                }
                if ui.button("Prepare").clicked() {
                    app.greeting = "Grace".into();
                }
            });
        });
    });
    let mut session = Session::new(adapter);
    session.settle(Instant::now() + Duration::from_secs(2));

    session
        .click(&Selector::role_named(accesskit::Role::Button, "Prepare"))
        .expect("click prepare");
    session
        .expect(Selector::text("Welcome, Grace").first())
        .to_be_visible()
        .expect("greeting appears after the state change");
}

/// The inspector overlay composites inside the pass (§13, M8): the
/// frame digest changes once the overlay session paints the highlight —
/// the "overlay on top of the app's scene at present time" contract,
/// observed through the same hash the stability checks consume.
#[test]
fn inspect_overlay_paints_into_the_pass() {
    use accesskit::Role;

    let (mut session, _state) = counter_session();

    // The digest before the overlay session: a static app re-presents
    // identical frames (same pass shape, no overlay shapes).
    let baseline = last_frame_digest(&mut session);

    // Highlight the Save button: the session hands the plan to the
    // adapter and the next pass composites outline + badges.
    let facts = session
        .inspect_highlight(&Selector::role_named(Role::Button, "Save"))
        .expect("highlight resolves");
    assert_eq!(facts.name.as_deref(), Some("Save"));
    assert!(
        session.adapter().overlay().is_some(),
        "the adapter holds the plan"
    );
    let with_overlay = last_frame_digest(&mut session);
    assert_ne!(
        baseline, with_overlay,
        "the overlay's outline + badge shapes changed the scene digest"
    );

    // Disabling stops compositing: the digest returns to the app-only
    // scene.
    session.inspect_disable_overlay().expect("overlay off");
    assert!(session.adapter().overlay().is_none());
    let after_disable = last_frame_digest(&mut session);
    assert_eq!(
        baseline, after_disable,
        "the scene is back to the app-only digest"
    );
}

/// The adapter's frame digest of one fresh pump (the §10 scene hash),
/// as integrated by the tree service.
fn last_frame_digest(session: &mut Session<EguiAdapter>) -> u64 {
    session.pump();
    session
        .tree()
        .last_frame()
        .map(|info| info.hash)
        .expect("every egui pump presents one frame")
}

// --- The §4.1 pixel path (m77) -------------------------------------------

use guiwright_driver::adapter::FrameRef;
use guiwright_driver::session::WireShot;

/// The pixel path is dual-behaved by host: a GPU host (CI linux rides
/// lavapipe, macOS/Windows have real GPUs) captures real pixels; a
/// GPU-less host answers `Unsupported` at the first pixel demand (the
/// cached probe failure, the masonry precedent). Both branches assert the
/// §6.2 contract — never a silent empty frame.
fn capture_window(session: &mut Session<EguiAdapter>) -> Result<WireShot, String> {
    session.screenshot_window().map_err(|e| e.to_string())
}

#[test]
fn caps_advertise_the_pixel_rows() {
    let (session, _state) = counter_session();
    let caps = &session.adapter().meta().caps;
    assert!(caps.render_frame, "m77 flips the render_frame row");
    assert!(caps.last_frame, "m77 flips the last_frame row");
}

#[test]
fn window_screenshot_is_real_pixels_or_gracefully_unsupported() {
    let (mut session, _state) = counter_session();

    match capture_window(&mut session) {
        Ok(shot) => {
            // The physical size of the default window rect (scale 1.0).
            assert_eq!(shot.width, 800, "physical px width");
            assert_eq!(shot.height, 600, "physical px height");
            assert!(!shot.png.is_empty(), "a real PNG body");
            // PNG magic + non-trivial payload (a blank frame still
            // compresses to almost nothing).
            assert_eq!(&shot.png[..8], b"\x89PNG\r\n\x1a\n", "png signature");
            assert!(shot.png.len() > 1_000, "encoded: {} bytes", shot.png.len());
        }
        Err(err) => {
            assert!(
                err.contains("render_frame") || err.contains("adapter"),
                "the honest unsupported row, got: {err}"
            );
        }
    }
}

#[test]
fn pixels_change_after_a_click_on_gpu_hosts() {
    fn frame_bytes(frame: &FrameRef) -> Vec<u8> {
        frame.pixels.as_ref().expect("pixels present").to_vec()
    }

    let (mut session, _state) = counter_session();
    let before = session.adapter_mut().render_frame(None);
    session.click(&save()).expect("click Save");
    let after = session.adapter_mut().render_frame(None);

    match (before, after) {
        (Ok(before), Ok(after)) => {
            let b = frame_bytes(&before);
            let a = frame_bytes(&after);
            assert_eq!(b.len(), a.len(), "same frame geometry");
            assert_ne!(b, a, "the click changed the pixels (counter text)");
        }
        (Err(e1), Err(e2)) => {
            // GPU-less host: both demands answer the honest row.
            assert!(e1.to_string().contains("render_frame"));
            assert!(e2.to_string().contains("render_frame"));
        }
        mixed => panic!("pixel path changed behavior mid-run: {mixed:?}"),
    }
}

#[test]
fn locator_screenshot_crops_the_button_on_gpu_hosts() {
    let (mut session, _state) = counter_session();
    let key = session.resolve(&save()).expect("resolve").key;
    match session.screenshot_element(&key) {
        Ok(shot) => {
            // The Save button crop: smaller than the window, real PNG.
            assert!(
                shot.width < 800 && shot.height < 600,
                "cropped: {}x{}",
                shot.width,
                shot.height
            );
            assert_eq!(&shot.png[..8], b"\x89PNG\r\n\x1a\n");
        }
        Err(e) => {
            assert!(e.to_string().contains("render_frame") || e.to_string().contains("adapter"));
        }
    }
}

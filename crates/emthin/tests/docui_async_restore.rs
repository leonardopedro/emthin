//! Regression: a saved page and zoom must survive the asynchronous relayout.
//!
//! Unit tests build `DocLayoutCache` with its synchronous test layout, so they
//! cannot see the race this covers. Integration tests link the real library,
//! where a text-size change hands layout to the worker; before the fix the page
//! was clamped against the old page count and never restored.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use emthin::docui::DocUi;
use serde_json::json;
use smithay::utils::Size;

fn temp_dir(name: &str) -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "emthin-docui-async-{name}-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn wait_current(ui: &mut DocUi) -> bool {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        while ui.poll_layout().is_some() {}
        if ui.geometry_current() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn stale_geometry_is_withheld_and_the_accepted_replacement_restores_it() {
    let dir = temp_dir("stale-geometry");
    let doc = dir.join("paper.typ");
    fs::write(
        &doc,
        "#1 visible caption #2 \\app(#1, #2, 300, 200)\n\nA line of prose.",
    )
    .expect("write document");

    let mut ui = DocUi::new();
    ui.set_viewport(Size::from((1200, 900)));
    ui.load(Some(&doc), None);
    assert!(wait_current(&mut ui), "initial layout did not arrive");

    let page = ui.layout().page_rect();
    let point = smithay::utils::Point::<f64, smithay::utils::Logical>::new(
        f64::from(page.loc.x + page.size.w / 2),
        f64::from(page.loc.y + page.size.h / 10),
    );
    assert!(ui.byte_at(point).is_some(), "the initial hit must be current");
    let figure = ui.figures().figures().first().cloned();
    if let Some(figure) = &figure {
        let centre = smithay::utils::Point::<f64, smithay::utils::Logical>::new(
            f64::from(figure.rect.loc.x + figure.rect.size.w / 2),
            f64::from(figure.rect.loc.y + figure.rect.size.h / 2),
        );
        assert!(ui.figure_at(centre).is_some(), "the figure is hittable initially");
    }

    // A multibyte insert changes every following byte offset. Until the worker
    // accepts a replacement, no source-derived hit may use the old glyph index.
    ui.model_mut().replace(0..0, "é ");
    assert!(!ui.geometry_current(), "the edit invalidates geometry immediately");
    assert!(ui.byte_at(point).is_none(), "no hit against stale offsets");
    assert!(ui.caret_rect().is_none(), "no IME caret against stale offsets");
    if let Some(figure) = &figure {
        let centre = smithay::utils::Point::<f64, smithay::utils::Logical>::new(
            f64::from(figure.rect.loc.x + figure.rect.size.w / 2),
            f64::from(figure.rect.loc.y + figure.rect.size.h / 2),
        );
        assert!(
            ui.figure_at(centre).is_none(),
            "a stale figure rect must not accept a launch/resize hit"
        );
    }
    assert!(ui.model().text().starts_with("é "), "keyboard editing continues");

    ui.relayout();
    assert!(wait_current(&mut ui), "the replacement did not restore geometry");
    assert!(ui.byte_at(point).is_some(), "the accepted replacement restores hits");

    // Open while a layout is in flight: the new document owns geometry, not
    // the result the worker still has queued.
    let other = dir.join("other.typ");
    fs::write(&other, "a different document").expect("other document");
    ui.model_mut().replace(0..0, "newer edit ");
    ui.relayout();
    ui.open_document(&other).expect("open while in flight");
    assert!(
        !ui.geometry_current(),
        "Open invalidates geometry before the new result arrives"
    );
    assert!(wait_current(&mut ui), "the opened document's layout did not arrive");

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn saved_page_and_zoom_survive_an_async_relayout() {
    let dir = temp_dir("saved-view");
    let doc = dir.join("paper.typ");
    let text =
        "A paragraph long enough that the fixture paginates at the default size.\n\n".repeat(200);
    fs::write(&doc, text).expect("write document");

    let key = fs::canonicalize(&doc)
        .expect("canonical document path")
        .to_string_lossy()
        .into_owned();
    let metadata = json!({
        "version": 1,
        "documents": {
            key: {
                "view_zoom": 2.0,
                "current_page": 2,
                "text_size": 20.0,
            },
        },
    });
    fs::write(dir.join("session.json"), metadata.to_string()).expect("write session metadata");

    let mut ui = DocUi::new();
    ui.set_viewport(Size::from((1200, 900)));
    // The session snapshot path need not exist when an explicit document wins;
    // its directory is what locates `session.json`.
    ui.load(Some(&doc), Some(&dir.join("session.loro")));

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut page_count = ui.page_count();
    while Instant::now() < deadline {
        while ui.poll_layout().is_some() {}
        page_count = ui.page_count();
        if page_count > 2 && ui.current_page() == 2 {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        page_count > 2,
        "the fixture laid out only {page_count} page(s); page restore is not distinct"
    );
    assert_eq!(
        ui.current_page(),
        2,
        "the saved page must be restored after the worker installs the new pages"
    );
    assert_eq!(ui.view_zoom(), 2.0, "the saved view zoom is restored");
    assert_eq!(ui.text_size(), 20.0, "the saved reading size is restored");

    let _ = fs::remove_dir_all(&dir);
}

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

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("emthin-docui-async-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[test]
fn saved_page_and_zoom_survive_an_async_relayout() {
    let dir = temp_dir();
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

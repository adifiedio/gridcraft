//! A host's copy of a workbook replaces what is open, and the session then reports its edits.

use egui::Shape;
use egui::epaint::ClippedShape;
use gridcraft_engine::Session;
use gridcraft_engine::core::CellRef;
use gridcraft_ui_egui::theme::Tokens;
use serde_json::json;

use crate::common;

fn harness(s: Session) -> common::Harness {
    common::harness_with(common::builder().with_size(egui::vec2(1200.0, 800.0)), common::app(s), 3)
}

#[test]
fn a_followed_workbook_replaces_what_was_open_and_logs_edits() {
    let mut source = Session::new();
    source.new_workbook();
    source.execute("cell.set", json!({"cell": "B2", "input": "42"})).unwrap();
    let bytes = gridcraft_engine::io::save_bytes(&source.active().unwrap().wb, "Book.xlsx").unwrap();
    let sources = json!([{"sheet": "Sheet1", "cell": "B2", "links": ["link-1"]}]);

    let mut h = harness(Session::new());
    let app = h.state_mut();
    app.open_followed("Model.xlsx", &bytes, sources.clone()).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    let d = app.session.active().unwrap();
    assert!(d.undo.is_empty() && !d.is_dirty());
    assert!(d.wb.active().unwrap().sources.contains_key(&CellRef::parse("B2").unwrap()));
    assert_eq!(app.session.edit_log.as_ref().map(Vec::len), Some(0));

    // Again: a reload after a refused edit leaves one workbook, and forgets what was unsent.
    app.session.run("cell.set", json!({"cell": "A1", "input": "1"})).unwrap();
    assert_eq!(app.session.edit_log.as_ref().map(Vec::len), Some(1));
    app.open_followed("Model.xlsx", &bytes, sources).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.session.edit_log.as_ref().map(Vec::len), Some(0));

    app.go_to("Sheet1", "B2").unwrap();
    assert_eq!(app.session.active().unwrap().selection.active.a1(), "B2");
    assert!(app.go_to("No such sheet", "B2").is_err());
}

/// Triangles in the accent colour on the page.
fn markers(h: &common::Harness) -> usize {
    let accent = Tokens::light().accent;
    let triangle = |shape: &ClippedShape| matches!(&shape.shape, Shape::Path(p) if p.points.len() == 3 && p.fill == accent);
    h.output().shapes.iter().filter(|s| triangle(s)).count()
}

#[test]
fn a_cell_with_a_source_shows_a_marker() {
    let mut s = Session::new();
    s.new_workbook();
    let mut h = harness(s);
    let before = markers(&h);
    h.state_mut().session.run("cell.set", json!({"cell": "B2", "input": "42"})).unwrap();
    h.state_mut().session.run("sources.set", json!({"entries": [{"sheet": "Sheet1", "cell": "B2", "links": ["l"]}]})).unwrap();
    h.run_steps(3);
    assert_eq!(markers(&h), before + 1);
}

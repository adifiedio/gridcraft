//! A session's edit log brings another session to the same workbook and the same undo history.

use gridcraft_engine::Session;
use serde_json::{Value as Json, json};

fn run(s: &mut Session, id: &str, params: Json) -> Json {
    s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn digest(s: &Session) -> String {
    s.active().map(|d| d.wb.content_digest()).unwrap_or_default()
}

/// Runs what `leader` logged on `follower`, and returns the commands it ran.
fn follow(leader: &mut Session, follower: &mut Session) -> Vec<String> {
    let log = leader.edit_log.as_mut().map(std::mem::take).unwrap_or_default();
    log.into_iter()
        .map(|edit| {
            run(follower, "selection.set", edit.selection);
            run(follower, &edit.command, edit.params);
            assert_eq!(digest(follower), edit.digest, "{} reached another workbook", edit.command);
            edit.command
        })
        .collect()
}

fn pair() -> (Session, Session) {
    let (mut leader, mut follower) = (Session::new(), Session::new());
    run(&mut leader, "file.new", json!({}));
    run(&mut follower, "file.new", json!({}));
    leader.edit_log = Some(Vec::new());
    (leader, follower)
}

#[test]
fn nothing_is_logged_unless_asked() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({}));
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "1"}));
    assert!(s.edit_log.is_none());
}

#[test]
fn selecting_and_reading_are_not_logged_and_travel_with_the_next_edit() {
    let (mut leader, mut follower) = pair();
    run(&mut leader, "selection.set", json!({"range": "B2:C3,E5", "active": "E5"}));
    run(&mut leader, "document.cells", json!({}));
    assert_eq!(leader.edit_log.as_ref().map(Vec::len), Some(0));
    run(&mut leader, "home.bold", json!({}));
    run(&mut leader, "edit.clearContents", json!({}));
    let log = leader.edit_log.clone().unwrap_or_default();
    let selection = log.first().map(|e| e.selection.clone()).unwrap_or_default();
    assert_eq!(selection["range"], "B2:C3,E5");
    assert_eq!(selection["active"], "E5");
    follow(&mut leader, &mut follower);
    assert_eq!(follower.active().map(|d| d.selection.clone()), leader.active().map(|d| d.selection.clone()));
}

#[test]
fn a_refused_command_is_not_logged() {
    let (mut leader, _) = pair();
    assert!(leader.execute("cell.set", json!({"cell": "not a cell", "input": "1"})).is_err());
    assert_eq!(leader.edit_log.as_ref().map(Vec::len), Some(0));
}

#[test]
fn only_the_outermost_command_is_logged() {
    let (mut leader, _) = pair();
    run(&mut leader, "range.setValues", json!({"range": "A1:B1", "values": [[1, 2]]}));
    run(&mut leader, "formulas.autoSum", json!({"range": "C1"}));
    let names: Vec<_> = leader.edit_log.iter().flatten().map(|e| e.command.clone()).collect();
    assert_eq!(names, ["range.setValues", "formulas.autoSum"]);
}

#[test]
fn copy_paste_undo_and_redo_follow() {
    let (mut leader, mut follower) = pair();
    run(&mut leader, "cell.set", json!({"cell": "A1", "input": "7"}));
    run(&mut leader, "selection.set", json!({"range": "A1"}));
    run(&mut leader, "edit.copy", json!({}));
    run(&mut leader, "selection.set", json!({"range": "C3"}));
    run(&mut leader, "edit.paste", json!({}));
    run(&mut leader, "edit.undo", json!({}));
    run(&mut leader, "edit.redo", json!({}));
    let ran = follow(&mut leader, &mut follower);
    assert_eq!(ran, ["cell.set", "edit.copy", "edit.paste", "edit.undo", "edit.redo"]);
    assert_eq!(digest(&follower), digest(&leader));
    assert_eq!(run(&mut follower, "document.cells", json!({})), run(&mut leader, "document.cells", json!({})));
}

#[test]
fn a_drags_merged_undo_step_follows() {
    let (mut leader, mut follower) = pair();
    run(&mut leader, "cell.set", json!({"cell": "A1", "input": "1"}));
    for (width, merged) in [(100, false), (120, true), (140, true)] {
        run(&mut leader, "home.columnWidth", json!({"cols": "A", "width": width}));
        assert_eq!(run(&mut leader, "edit.coalesceUndo", json!({}))["merged"], merged);
    }
    follow(&mut leader, &mut follower);
    let depth = |s: &Session| s.active().map(|d| d.undo.len());
    assert_eq!(depth(&follower), depth(&leader));
    assert_eq!(depth(&leader), Some(2));
}

#[test]
fn work_on_another_sheet_follows() {
    let (mut leader, mut follower) = pair();
    run(&mut leader, "home.insertSheet", json!({"name": "Second, with a comma"}));
    run(&mut leader, "cell.set", json!({"cell": "B2", "input": "=1+1"}));
    run(&mut leader, "sheet.previous", json!({}));
    run(&mut leader, "cell.set", json!({"cell": "A1", "input": "first"}));
    follow(&mut leader, &mut follower);
    assert_eq!(digest(&follower), digest(&leader));
}

#[test]
fn a_tables_filter_turns_on_outright_and_undoes() {
    let (mut leader, mut follower) = pair();
    run(&mut leader, "range.setValues", json!({"range": "A1:B3", "values": [["a", "b"], [1, 2], [3, 4]]}));
    run(&mut leader, "insert.table", json!({"range": "A1:B3"}));
    run(&mut leader, "selection.set", json!({"range": "A2"}));
    let on = |s: &Session| s.active().and_then(|d| d.wb.active()).is_some_and(|sh| sh.autofilter.is_some());
    run(&mut leader, "data.filter", json!({"range": "A1:B3", "install": true}));
    assert!(on(&leader));
    follow(&mut leader, &mut follower);
    assert!(on(&follower));
    run(&mut leader, "edit.undo", json!({}));
    assert!(!on(&leader));
}

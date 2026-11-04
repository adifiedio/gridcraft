//! `document.cells` gives a host everything occupied, without asking for each range.

use gridcraft_engine::Session;
use serde_json::{Value as Json, json};

fn run(s: &mut Session, id: &str, params: Json) -> Json {
    s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

#[test]
fn lists_values_formulas_styles_and_structure() {
    let mut s = Session::new();
    s.new_workbook();
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "10"}));
    run(&mut s, "cell.set", json!({"cell": "B1", "input": "=A1*2"}));
    run(&mut s, "cell.set", json!({"cell": "C1", "input": "=1/0"}));
    run(&mut s, "home.numberFormat", json!({"range": "A1", "code": "0.00"}));
    run(&mut s, "home.bold", json!({"range": "A1", "on": true}));
    run(&mut s, "home.mergeCells", json!({"range": "E5:F5"}));
    run(&mut s, "home.insertSheet", json!({"name": "Second Sheet"}));
    let all = run(&mut s, "document.cells", json!({}));

    let first = &all["sheets"][0];
    assert_eq!(first["merges"], json!(["E5:F5"]));
    let cells = first["cells"].as_array().unwrap();
    let at = |row: u64, col: u64| cells.iter().find(|c| c["r"] == row && c["c"] == col).unwrap();
    assert_eq!(at(0, 0)["v"], json!(10.0));
    assert_eq!(at(0, 1)["f"], json!("=A1*2"));
    assert_eq!(at(0, 1)["v"], json!(20.0));
    assert_eq!(at(0, 2)["v"], json!({"error": "#DIV/0!"}));
    let style = &all["styles"][at(0, 0)["s"].to_string()];
    assert_eq!(style["numFmt"], json!("0.00"));
    assert_eq!(style["bold"], json!(true));
    assert_eq!(all["sheets"][1]["name"], json!("Second Sheet"));
}

#[test]
fn marks_what_a_formula_spills() {
    let mut s = Session::new();
    s.new_workbook();
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "=SEQUENCE(3)"}));
    let all = run(&mut s, "document.cells", json!({}));
    let cells = all["sheets"][0]["cells"].as_array().unwrap();
    assert_eq!(cells.len(), 3);
    assert!(cells.iter().all(|c| c["spill"] == json!("A1")));
    assert_eq!(cells.iter().filter(|c| c.get("f").is_some()).count(), 1);
}

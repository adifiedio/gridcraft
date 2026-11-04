//! Cell sources follow their cells through everything that moves them, and go when the content
//! they describe goes. The fuzz test gives every sourced cell a value of its own, so at any moment
//! a cell's value says which link it must carry.

use std::collections::BTreeMap;

use gridcraft_engine::{EngineError, Session};
use serde_json::{Value as Json, json};

const LITERALS: i64 = 1000;
const FORMULAS: i64 = 5000;

fn new_session() -> Session {
    let mut s = Session::new();
    s.new_workbook();
    s
}

fn run(s: &mut Session, id: &str, params: Json) -> Json {
    s.execute(id, params).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn links(s: &mut Session, cell: &str) -> Json {
    run(s, "cell.get", json!({"cell": cell}))["sources"].clone()
}

fn sourced() -> Session {
    let mut s = new_session();
    run(&mut s, "range.setValues", json!({"range": "A1:B3", "values": [[3, 30], [1, 10], [2, 20]]}));
    run(
        &mut s,
        "sources.set",
        json!({"entries": [
            {"cell": "A1", "links": ["three"]},
            {"cell": "B2", "links": ["ten", "ten-bis"]},
        ]}),
    );
    s
}

#[test]
fn set_and_list() {
    let mut s = sourced();
    assert_eq!(links(&mut s, "B2"), json!(["ten", "ten-bis"]));
    assert_eq!(links(&mut s, "A2"), Json::Null);
    let listed = run(&mut s, "sources.list", json!({}));
    assert_eq!(listed["sources"][0], json!({"sheet": "Sheet1", "cell": "A1", "links": ["three"], "value": 3.0, "formula": null}));
    run(&mut s, "sources.set", json!({"entries": [{"cell": "A1", "links": []}]}));
    assert_eq!(links(&mut s, "A1"), Json::Null);
}

#[test]
fn bad_entries_change_nothing() {
    let mut s = sourced();
    for bad in [
        json!({}),
        json!({"entries": [{"links": ["a"]}]}),
        json!({"entries": [{"cell": "A1"}]}),
        json!({"entries": [{"cell": "A1", "links": ["no spaces"]}]}),
        json!({"entries": [{"cell": "A1", "links": [""]}]}),
        json!({"entries": [{"cell": "A1", "links": [7]}]}),
        json!({"entries": [{"cell": "A1", "links": ["x".repeat(129)]}]}),
        json!({"entries": [{"cell": "A1", "links": vec!["x"; 65].iter().enumerate().map(|(i, _)| format!("x{i}")).collect::<Vec<_>>()}]}),
        json!({"entries": [{"cell": "nope", "links": ["a"]}]}),
        json!({"entries": [{"cell": "A1", "sheet": "Missing", "links": ["a"]}]}),
        // The first entry is fine, the second names a blank cell: neither is applied.
        json!({"entries": [{"cell": "A2", "links": ["a"]}, {"cell": "Z9", "links": ["a"]}]}),
    ] {
        assert!(s.execute("sources.set", bad.clone()).is_err(), "{bad}");
        assert_eq!(links(&mut s, "A2"), Json::Null, "{bad}");
        assert_eq!(links(&mut s, "A1"), json!(["three"]), "{bad}");
    }
}

#[test]
fn sort_carries_sources_and_undo_brings_them_back() {
    let mut s = sourced();
    run(&mut s, "data.sort", json!({"range": "A1:B3", "keys": [{"column": "A", "order": "asc"}], "header": false}));
    assert_eq!(links(&mut s, "A3"), json!(["three"]));
    assert_eq!(links(&mut s, "B1"), json!(["ten", "ten-bis"]));
    assert_eq!(links(&mut s, "A1"), Json::Null);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(links(&mut s, "A1"), json!(["three"]));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(links(&mut s, "A3"), json!(["three"]));
}

#[test]
fn inserts_and_deletes_move_or_remove_sources() {
    let mut s = sourced();
    run(&mut s, "home.insertRows", json!({"rows": "1:1"}));
    assert_eq!(links(&mut s, "A2"), json!(["three"]));
    run(&mut s, "home.insertColumns", json!({"cols": "A:A"}));
    assert_eq!(links(&mut s, "B2"), json!(["three"]));
    run(&mut s, "home.insertCells", json!({"range": "B2", "shift": "right"}));
    assert_eq!(links(&mut s, "C2"), json!(["three"]));
    run(&mut s, "home.deleteCells", json!({"range": "B2", "shift": "left"}));
    assert_eq!(links(&mut s, "B2"), json!(["three"]));
    run(&mut s, "home.deleteRows", json!({"rows": "2:2"}));
    assert_eq!(run(&mut s, "sources.list", json!({}))["sources"].as_array().map(Vec::len), Some(1));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(links(&mut s, "B2"), json!(["three"]));
}

#[test]
fn overwriting_removes_sources_and_typing_the_same_value_does_not() {
    let mut s = sourced();
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "3"}));
    assert_eq!(links(&mut s, "A1"), json!(["three"]));
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "4"}));
    assert_eq!(links(&mut s, "A1"), Json::Null);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(links(&mut s, "A1"), json!(["three"]));
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "=1+2"}));
    assert_eq!(links(&mut s, "A1"), Json::Null, "a formula is not the value it replaced");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.clearContents", json!({"range": "A1"}));
    assert_eq!(links(&mut s, "A1"), Json::Null);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.clearFormats", json!({"range": "A1"}));
    assert_eq!(links(&mut s, "A1"), json!(["three"]), "formats are not content");
}

#[test]
fn copies_keep_sources_while_the_value_matches() {
    let mut s = sourced();
    run(&mut s, "edit.copy", json!({"range": "A1"}));
    run(&mut s, "selection.set", json!({"range": "D1"}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(links(&mut s, "D1"), json!(["three"]));
    assert_eq!(links(&mut s, "A1"), json!(["three"]), "the original keeps its sources");
    run(&mut s, "selection.set", json!({"range": "D2"}));
    run(&mut s, "edit.pasteSpecial", json!({"what": "values"}));
    assert_eq!(links(&mut s, "D2"), json!(["three"]));
    run(&mut s, "selection.set", json!({"range": "D3"}));
    run(&mut s, "edit.pasteSpecial", json!({"what": "formats"}));
    assert_eq!(links(&mut s, "D3"), Json::Null);
    run(&mut s, "cell.set", json!({"cell": "D4", "input": "5"}));
    run(&mut s, "selection.set", json!({"range": "D4"}));
    run(&mut s, "edit.pasteSpecial", json!({"what": "values", "operation": "add"}));
    assert_eq!(run(&mut s, "cell.get", json!({"cell": "D4"}))["value"], json!(8.0));
    assert_eq!(links(&mut s, "D4"), Json::Null, "a sum is not the sourced value");
}

#[test]
fn a_cut_moves_sources() {
    let mut s = sourced();
    run(&mut s, "edit.cut", json!({"range": "A1"}));
    run(&mut s, "selection.set", json!({"range": "D5"}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(links(&mut s, "D5"), json!(["three"]));
    assert_eq!(links(&mut s, "A1"), Json::Null);
}

/// Regression: a sort between a cut and its paste moved other cells into the cut block, and the
/// paste took the cells of the moment with the sources of the moment of the cut.
#[test]
fn a_cut_pasted_after_a_sort_takes_the_sources_of_what_it_moves() {
    let mut s = sourced();
    run(&mut s, "edit.cut", json!({"range": "A1"}));
    run(&mut s, "data.sort", json!({"range": "A1:B3", "keys": [{"column": "A", "order": "asc"}], "header": false}));
    run(&mut s, "selection.set", json!({"range": "D5"}));
    run(&mut s, "edit.paste", json!({}));
    assert_eq!(run(&mut s, "cell.get", json!({"cell": "D5"}))["value"], json!(1.0));
    assert_eq!(links(&mut s, "D5"), Json::Null);
    assert_eq!(links(&mut s, "A3"), json!(["three"]));
}

#[test]
fn fill_keeps_sources_only_on_equal_values() {
    let mut s = new_session();
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "7"}));
    run(&mut s, "cell.set", json!({"cell": "C1", "input": "=A1*2"}));
    run(&mut s, "cell.set", json!({"cell": "A2", "input": "8"}));
    run(&mut s, "sources.set", json!({"entries": [{"cell": "A1", "links": ["a"]}, {"cell": "C1", "links": ["c"]}]}));
    run(&mut s, "edit.fillRight", json!({"range": "A1:B1"}));
    assert_eq!(links(&mut s, "B1"), json!(["a"]));
    run(&mut s, "edit.fillDown", json!({"range": "C1:C2"}));
    assert_eq!(run(&mut s, "cell.get", json!({"cell": "C2"}))["value"], json!(16.0));
    assert_eq!(links(&mut s, "C2"), Json::Null, "the copied formula calculates something else");
}

#[test]
fn a_formula_keeps_sources_when_its_references_shift_or_its_value_recalculates() {
    let mut s = new_session();
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "2"}));
    run(&mut s, "cell.set", json!({"cell": "B5", "input": "=A1*10"}));
    run(&mut s, "sources.set", json!({"entries": [{"cell": "B5", "links": ["model"]}]}));
    run(&mut s, "cell.set", json!({"cell": "A1", "input": "3"}));
    assert_eq!(links(&mut s, "B5"), json!(["model"]), "a recalculation is not an edit");
    run(&mut s, "home.insertRows", json!({"rows": "1:1"}));
    assert_eq!(run(&mut s, "cell.get", json!({"cell": "B6"}))["formula"], "=A2*10");
    assert_eq!(links(&mut s, "B6"), json!(["model"]));
    run(&mut s, "cell.set", json!({"cell": "B6", "input": "=A3*10"}));
    assert_eq!(links(&mut s, "B6"), Json::Null, "pointing it elsewhere by hand is an edit");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "cell.set", json!({"cell": "B6", "input": "=A2*20"}));
    assert_eq!(links(&mut s, "B6"), Json::Null);
}

#[test]
fn a_copied_sheet_keeps_sources_and_the_file_has_none() {
    let mut s = sourced();
    run(&mut s, "sheet.move", json!({"sheet": "Sheet1", "to": 1, "copy": true}));
    let listed = run(&mut s, "sources.list", json!({}));
    assert_eq!(listed["sources"].as_array().map(Vec::len), Some(4));
    let d = s.doc().unwrap();
    let bytes = gridcraft_engine::io::save_bytes(&d.wb, "x.xlsx").unwrap();
    let mut reopened = Session::new();
    reopened.execute("file.open", json!({"base64": base64_of(&bytes), "name": "x.xlsx"})).unwrap();
    assert_eq!(run(&mut reopened, "sources.list", json!({}))["sources"], json!([]));
}

fn base64_of(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            out.push(if i <= chunk.len() { T[(n >> (18 - 6 * i)) as usize & 63] as char } else { '=' });
        }
    }
    out
}

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn upto(&mut self, n: u64) -> u32 {
        (self.next() % n) as u32
    }
    fn cell(&mut self) -> String {
        format!("{}{}", char::from(b'A' + self.upto(7) as u8), 1 + self.upto(24))
    }
    fn range(&mut self) -> String {
        let (c0, c1) = (self.upto(6), self.upto(3));
        let (r0, r1) = (1 + self.upto(20), self.upto(5));
        format!("{}{}:{}{}", char::from(b'A' + c0 as u8), r0, char::from(b'A' + (c0 + c1) as u8), r0 + r1)
    }
}

/// Every cell holding one of the fuzz's own values has that value's link, and only that one;
/// every entry sits on a cell with its link's value.
fn check(s: &Session, step: &str) {
    let d = s.doc().unwrap();
    for sh in &d.wb.sheets {
        let mut carried = BTreeMap::new();
        for (c, cell) in sh.cells.iter() {
            let gridcraft_core::Value::Number(n) = cell.value else { continue };
            let n = n as i64;
            if (LITERALS..LITERALS + 200).contains(&n) || (FORMULAS..FORMULAS + 200).contains(&n) {
                let got = sh.sources.get(&c).map(|e| e.links.clone());
                assert_eq!(got, Some(vec![format!("v{n}")]), "{} {} holds {n} after {step}", sh.name, c.a1());
                carried.insert(c, n);
            }
        }
        for (c, entry) in &sh.sources {
            assert!(carried.contains_key(c), "{} {} kept {:?} after {step}, holding {:?}", sh.name, c.a1(), entry.links, sh.value(*c));
        }
    }
}

fn entries(s: &Session) -> Vec<(String, gridcraft_core::CellRef, Vec<String>)> {
    let d = s.doc().unwrap();
    let mut all = Vec::new();
    for sh in &d.wb.sheets {
        all.extend(sh.sources.iter().map(|(c, e)| (sh.name.clone(), *c, e.links.clone())));
    }
    all
}

fn fuzz(seed: u64, steps: usize) {
    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let mut s = new_session();
    let mut attach = Vec::new();
    let mut n = 0;
    for row in 1..=20 {
        for col in ['A', 'B', 'C', 'D', 'E'] {
            let cell = format!("{col}{row}");
            let value = if (row + n) % 5 == 0 { FORMULAS + n } else { LITERALS + n };
            let input = if value >= FORMULAS { format!("=$AA$100+{value}") } else { value.to_string() };
            run(&mut s, "cell.set", json!({"cell": cell, "input": input}));
            attach.push(json!({"cell": cell, "links": [format!("v{value}")]}));
            n += 1;
        }
    }
    run(&mut s, "sources.set", json!({"entries": attach}));
    check(&s, "setup");
    let start = entries(&s);
    // The fuzz undoes back to here and no further.
    s.doc_mut().unwrap().undo.clear();
    for step in 0..steps {
        let (id, params) = match rng.upto(18) {
            0 => {
                let column = char::from(b'A' + rng.upto(5) as u8).to_string();
                (
                    "data.sort",
                    json!({"range": "A1:E20", "keys": [{"column": column, "order": if rng.upto(2) == 0 { "asc" } else { "desc" }}], "header": false}),
                )
            }
            1 => ("home.insertRows", json!({"rows": format!("{0}:{0}", 1 + rng.upto(20))})),
            2 => ("home.deleteRows", json!({"rows": format!("{0}:{0}", 1 + rng.upto(20))})),
            3 => ("home.insertColumns", json!({"cols": "B:B"})),
            4 => ("home.deleteColumns", json!({"cols": "C:C"})),
            5 => ("home.insertCells", json!({"range": rng.range(), "shift": if rng.upto(2) == 0 { "right" } else { "down" }})),
            6 => ("home.deleteCells", json!({"range": rng.range(), "shift": if rng.upto(2) == 0 { "left" } else { "up" }})),
            7 => ("edit.copy", json!({"range": rng.range()})),
            8 => ("edit.cut", json!({"range": rng.range()})),
            9 => {
                run(&mut s, "selection.set", json!({"range": rng.cell()}));
                if rng.upto(2) == 0 { ("edit.paste", json!({})) } else { ("edit.pasteSpecial", json!({"what": "values"})) }
            }
            10 => ("cell.set", json!({"cell": rng.cell(), "input": "-1"})),
            11 => ("edit.clearContents", json!({"range": rng.range()})),
            12 => ("data.removeDuplicates", json!({"range": rng.range()})),
            13 => ("sheet.moveColumns", json!({"cols": "B:C", "before": rng.upto(7)})),
            14 => ("edit.fillDown", json!({"range": rng.range()})),
            15 => ("sheet.move", json!({"sheet": 0, "to": 1, "copy": true})),
            16 => ("edit.undo", json!({})),
            _ => ("edit.redo", json!({})),
        };
        let label = format!("seed {seed} step {step}: {id} {params}");
        match s.execute(id, params) {
            Err(EngineError::Internal(id, msg)) => panic!("{label}: {id} panicked: {msg}"),
            _ => check(&s, &label),
        }
    }
    while !s.doc().unwrap().undo.is_empty() {
        run(&mut s, "edit.undo", json!({}));
        check(&s, "unwinding");
    }
    assert_eq!(entries(&s), start, "seed {seed}: undoing everything restores the sources");
}

#[test]
fn sources_follow_cells_through_random_edits() {
    let var = |name: &str, default: u64| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    if let Ok(seed) = std::env::var("GRIDCRAFT_FUZZ_SEED") {
        fuzz(seed.parse().unwrap(), var("GRIDCRAFT_FUZZ_STEPS", 60) as usize);
        return;
    }
    for seed in 1..=var("GRIDCRAFT_FUZZ_SEEDS", 12) {
        fuzz(seed, var("GRIDCRAFT_FUZZ_STEPS", 60) as usize);
    }
}

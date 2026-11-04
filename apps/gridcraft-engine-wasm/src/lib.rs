//! The engine with no UI, for a host running it as WebAssembly: commands in, results out, as JSON.
//! The module has no file or network access; `Engine::save` and `Engine::open` carry the bytes.
//! A panic traps and leaves the instance unusable, so a host discards it and loads the last save.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use gridcraft_engine::{DocState, Session, Trust, io};
use serde_json::{Value as Json, json};
use wasm_bindgen::prelude::*;

const FILE_NAME: &str = "workbook.xlsx";

#[wasm_bindgen]
pub struct Engine {
    session: Session,
    warnings: Vec<String>,
}

#[wasm_bindgen]
impl Engine {
    /// A blank workbook.
    #[wasm_bindgen(constructor)]
    #[allow(clippy::new_without_default)]
    pub fn new() -> Engine {
        let mut session = Session::new();
        session.new_workbook();
        Engine { session, warnings: Vec::new() }
    }

    /// An XLSX file's workbook, calculated afresh, since a file's cached values may not be what its
    /// formulas give. `strict` refuses a file that loads with warnings: a save of ours should have
    /// none, and one that loads incomplete must not replace the good copy.
    pub fn open(bytes: &[u8], strict: bool) -> Result<Engine, String> {
        // Anything else would open as a CSV: every text is a valid one.
        if !bytes.starts_with(b"PK\x03\x04") {
            return Err("not an XLSX file".into());
        }
        let (wb, warnings) = io::open_bytes(FILE_NAME, bytes).map_err(|e| e.to_string())?;
        if strict && !warnings.is_empty() {
            return Err(format!("the saved workbook did not load completely: {}", warnings.join("; ")));
        }
        let mut session = Session::new();
        session.add_document(DocState::new(wb, None, FILE_NAME.into(), Trust::FromFile));
        Ok(Engine { session, warnings })
    }

    /// What opening the file warned about, as a JSON array.
    pub fn warnings(&self) -> String {
        json!(self.warnings).to_string()
    }

    /// Runs one command; `params` is JSON, and so is the result.
    pub fn exec(&mut self, id: &str, params: &str) -> Result<String, String> {
        let params = serde_json::from_str(params).map_err(|e| e.to_string())?;
        self.session.execute_headless(id, params).map(|r| r.to_string()).map_err(|e| e.to_string())
    }

    /// Runs `[{"command": id, "params": {…}}, …]` as one undo step, all or nothing.
    pub fn batch(&mut self, label: &str, commands: &str) -> Result<String, String> {
        let commands: Vec<Json> = serde_json::from_str(commands).map_err(|e| e.to_string())?;
        let results = self.session.grouped(label, |s| {
            let mut results = Vec::with_capacity(commands.len());
            for c in &commands {
                let id = c.get("command").and_then(Json::as_str).unwrap_or_default();
                results.push(s.execute_headless(id, c.get("params").cloned().unwrap_or(Json::Null))?);
            }
            Ok(results)
        });
        results.map(|r| json!(r).to_string()).map_err(|e| e.to_string())
    }

    /// Puts back the sources saved beside a file (`sources.set` params). Opening a file is not an
    /// edit, so this leaves nothing to undo.
    #[wasm_bindgen(js_name = restoreSources)]
    pub fn restore_sources(&mut self, params: &str) -> Result<(), String> {
        self.exec("sources.set", params)?;
        let doc = self.session.doc_mut().map_err(|e| e.to_string())?;
        doc.undo.clear();
        doc.redo.clear();
        Ok(())
    }

    /// The workbook as an XLSX file. Sources are not in it: `sources.list` gives them.
    pub fn save(&self) -> Result<Vec<u8>, String> {
        let doc = self.session.doc().map_err(|e| e.to_string())?;
        io::save_bytes(&doc.wb, FILE_NAME).map_err(|e| e.to_string())
    }

    /// See `Workbook::content_digest`.
    pub fn digest(&self) -> Result<String, String> {
        Ok(self.session.doc().map_err(|e| e.to_string())?.wb.content_digest())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(e: &mut Engine, id: &str, params: Json) -> Result<Json, String> {
        e.exec(id, &params.to_string()).and_then(|r| serde_json::from_str(&r).map_err(|err| err.to_string()))
    }

    fn cell(e: &mut Engine, at: &str, key: &str) -> Result<Json, String> {
        Ok(run(e, "cell.get", json!({"cell": at}))?[key].clone())
    }

    fn sample() -> Result<Engine, String> {
        let mut e = Engine::new();
        run(&mut e, "range.setValues", json!({"range": "A1:B2", "values": [[1, "two"], [3.5, true]]}))?;
        run(&mut e, "cell.set", json!({"cell": "C1", "input": "=A1+A2"}))?;
        run(&mut e, "cell.set", json!({"cell": "C2", "input": "=NOW()"}))?;
        run(&mut e, "cell.set", json!({"cell": "C3", "input": "=SUM(A1:A2)*RAND()"}))?;
        run(&mut e, "sources.set", json!({"entries": [{"cell": "A1", "links": ["doc-1"]}, {"cell": "C1", "links": ["doc-2"]}]}))?;
        Ok(e)
    }

    #[test]
    fn a_batch_is_one_undo_step_and_all_or_nothing() -> Result<(), String> {
        let mut e = Engine::new();
        let batch = json!([
            {"command": "cell.set", "params": {"cell": "A1", "input": "1"}},
            {"command": "cell.set", "params": {"cell": "A2", "input": "2"}},
        ]);
        e.batch("Two", &batch.to_string())?;
        assert_eq!(cell(&mut e, "A2", "value")?, json!(2.0));
        run(&mut e, "edit.undo", json!({}))?;
        assert_eq!(cell(&mut e, "A1", "value")?, Json::Null);

        let failing = json!([
            {"command": "cell.set", "params": {"cell": "A1", "input": "1"}},
            {"command": "no.such.command", "params": {}},
        ]);
        assert!(e.batch("Bad", &failing.to_string()).is_err());
        assert_eq!(cell(&mut e, "A1", "value")?, Json::Null);
        Ok(())
    }

    #[test]
    fn a_save_reopens_to_the_same_digest() -> Result<(), String> {
        let e = sample()?;
        let bytes = e.save()?;
        let mut back = Engine::open(&bytes, true)?;
        let sources = {
            let mut original = sample()?;
            run(&mut original, "sources.list", json!({}))?["sources"].clone()
        };
        back.restore_sources(&json!({"entries": sources}).to_string())?;
        assert_eq!(back.digest()?, e.digest()?);
        assert_eq!(cell(&mut back, "C1", "value")?, json!(4.5));
        assert!(back.exec("edit.undo", "{}").is_err(), "reopening leaves nothing to undo");
        Ok(())
    }

    #[test]
    fn a_digest_sees_content_and_sources_only() -> Result<(), String> {
        let mut e = sample()?;
        let start = e.digest()?;
        run(&mut e, "formulas.calculateNow", json!({}))?;
        run(&mut e, "home.bold", json!({"range": "A1"}))?;
        assert_eq!(e.digest()?, start, "volatile values and formats do not count");
        run(&mut e, "sources.set", json!({"entries": [{"cell": "A1", "links": []}]}))?;
        let unsourced = e.digest()?;
        assert_ne!(unsourced, start);
        run(&mut e, "cell.set", json!({"cell": "A1", "input": "2"}))?;
        assert_ne!(e.digest()?, unsourced);
        Ok(())
    }

    #[test]
    fn garbage_is_refused() {
        assert!(Engine::open(b"not a workbook", false).is_err());
        let mut e = Engine::new();
        assert!(e.exec("cell.set", "{not json").is_err());
        assert!(e.batch("x", "{}").is_err());
    }
}

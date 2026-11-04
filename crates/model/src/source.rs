//! Cell sources: which evidence a cell's content came from.
//!
//! A cell lists the ids of its source links; the records behind the ids live with the host. An
//! entry sits beside the cell, as a note does, and moves with it through sorts, inserts, deletes
//! and cuts. Copies take the entry along while the copy's value matches. After every edit
//! [`Workbook::refresh_sources`] removes the entries whose cell no longer holds what was attached,
//! so whichever command overwrites a cell, it cannot keep a source that no longer applies.

use std::sync::Arc;

use gridcraft_core::Value;
use gridcraft_formula::{Expr, parse};
use serde::{Deserialize, Serialize};

use crate::{Cell, Formula, Workbook};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellSource {
    pub links: Vec<String>,
    /// The value when attached. A formula's value may change later without affecting its sources.
    pub value: Value,
    /// The formula text when attached, kept in step as its references move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    /// Written by a copy in this edit, and to be checked once the copy has calculated.
    #[serde(skip)]
    pending_copy: bool,
}

impl CellSource {
    /// Sources for what `cell` holds now. A blank or Python cell cannot have any.
    pub fn attach(links: Vec<String>, cell: &Cell) -> Option<CellSource> {
        if cell.py.is_some() || (cell.formula.is_none() && cell.value.is_empty()) {
            return None;
        }
        Some(CellSource { links, value: cell.value.clone(), formula: cell.formula.as_ref().map(|f| f.text.clone()), pending_copy: false })
    }

    /// The entry for a copy of a cell whose value is `value` now.
    pub fn copy_with(&self, value: Value) -> CellSource {
        CellSource { links: self.links.clone(), value, formula: None, pending_copy: true }
    }

    /// What an edit did to this entry, given the cell now at its address: `None` if nothing,
    /// `Some(None)` if the entry goes, and `Some(Some(_))` if it is replaced.
    fn after_edit(&self, cell: Option<&Cell>, refs_moved: bool) -> Option<Option<CellSource>> {
        let Some(cell) = cell else { return Some(None) };
        if self.pending_copy {
            return Some(if cell.value == self.value { CellSource::attach(self.links.clone(), cell) } else { None });
        }
        if cell.py.is_some() {
            return Some(None);
        }
        match (&self.formula, &cell.formula) {
            (None, None) if cell.value == self.value => None,
            (Some(old), Some(new)) if *old == new.text => None,
            (Some(old), Some(new)) if refs_moved && same_but_references(old, new) => {
                Some(Some(CellSource { formula: Some(new.text.clone()), ..self.clone() }))
            }
            _ => Some(None),
        }
    }
}

/// Whether two formulas differ only in where their references point. A formula's text changes
/// that way when cells move, which is not an edit of the formula.
fn same_but_references(old: &str, new: &Formula) -> bool {
    let shape = |e: Expr| e.map(&mut |x| if matches!(x, Expr::Ref(_)) { Expr::Missing } else { x });
    match (parse(old), new.expr()) {
        (Ok(a), Some(b)) => shape(a) == shape(b),
        _ => false,
    }
}

impl Workbook {
    /// Brings the sources in line with the cells after an edit, calculation included. `refs_moved`
    /// says the edit may have rewritten references in formulas it did not otherwise change.
    pub fn refresh_sources(&mut self, refs_moved: bool) {
        for sheet in &mut self.sheets {
            let changes: Vec<_> = sheet.sources.iter().filter_map(|(c, s)| s.after_edit(sheet.cell(*c), refs_moved).map(|next| (*c, next))).collect();
            if changes.is_empty() {
                continue;
            }
            let sheet = Arc::make_mut(sheet);
            for (c, next) in changes {
                match next {
                    Some(s) => sheet.sources.insert(c, s),
                    None => sheet.sources.remove(&c),
                };
            }
        }
    }
}

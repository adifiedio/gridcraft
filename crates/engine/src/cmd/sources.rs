//! Cell sources: attach evidence link ids to cells, and read them back. A host that does not trust
//! its users keeps `sources.set` to itself, so only it can say where a value came from.

use gridcraft_model::CellSource;
use serde_json::{Value as Json, json};

use super::{CommandSpec, Result, Session, bad, cell_param_on, cmd, edit, has_doc, target_sheet};

const MAX_ENTRIES: usize = 1_000_000;
const MAX_LINKS_PER_CELL: usize = 64;
const MAX_LINK_ID_LEN: usize = 128;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "sources.set",
            "Set Cell Sources",
            [],
            None,
            "{entries: [{cell, sheet?, links: [id]}]}: sources for what each cell holds now; no links removes them",
            has_doc,
            set
        ),
        cmd!(query "sources.list", "List Cell Sources", [], None, "{sheet?} → sources: [{sheet, cell, links, value, formula}]", has_doc, list),
    ]
}

fn link_ids(entry: &Json) -> Result<Vec<String>> {
    let ids = entry.get("links").and_then(Json::as_array).ok_or_else(|| bad("links", "expected an array of link ids"))?;
    if ids.len() > MAX_LINKS_PER_CELL {
        return Err(bad("links", format!("at most {MAX_LINKS_PER_CELL} links per cell")));
    }
    let mut links = Vec::with_capacity(ids.len());
    for id in ids {
        let id = id
            .as_str()
            .filter(|t| !t.is_empty() && t.len() <= MAX_LINK_ID_LEN && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'));
        let id = id.ok_or_else(|| bad("links", format!("a link id is 1 to {MAX_LINK_ID_LEN} letters, digits, `-` or `_`")))?;
        if !links.iter().any(|l| l == id) {
            links.push(id.to_string());
        }
    }
    Ok(links)
}

fn set(s: &mut Session, p: &Json) -> Result<Json> {
    let entries = p.get("entries").and_then(Json::as_array).ok_or_else(|| bad("entries", "expected an array"))?;
    if entries.len() > MAX_ENTRIES {
        return Err(bad("entries", format!("at most {MAX_ENTRIES} entries")));
    }
    let mut parsed = Vec::with_capacity(entries.len());
    for entry in entries {
        let sheet = target_sheet(s, entry)?;
        let cell = cell_param_on(s, entry, "cell", sheet)?.ok_or_else(|| bad("cell", "each entry needs a cell"))?;
        parsed.push((sheet, cell, link_ids(entry)?));
    }
    edit(s, |cx| {
        for (sheet, cell, links) in parsed {
            let sh = cx.sheet_mut(sheet)?;
            if links.is_empty() {
                sh.sources.remove(&cell);
                continue;
            }
            let attached = sh.cell(cell).and_then(|c| CellSource::attach(links, c));
            let attached = attached.ok_or_else(|| bad("cell", format!("{} holds nothing a source can describe", cell.a1())))?;
            sh.sources.insert(cell, attached);
        }
        Ok(Json::Null)
    })
}

fn list(s: &mut Session, p: &Json) -> Result<Json> {
    let only = if p.get("sheet").is_some() { Some(target_sheet(s, p)?) } else { None };
    let wb = &s.doc()?.wb;
    let sources: Vec<Json> = wb
        .sheets
        .iter()
        .enumerate()
        .filter(|(i, _)| only.is_none_or(|o| o == *i))
        .flat_map(|(_, sh)| {
            sh.sources.iter().map(|(c, src)| {
                json!({"sheet": sh.name, "cell": c.a1(), "links": src.links, "value": super::inspect::value_json(&src.value), "formula": src.formula})
            })
        })
        .collect();
    Ok(json!({"sources": sources}))
}

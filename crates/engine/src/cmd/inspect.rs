//! Read-only commands for agents: inspect the workbook, read ranges, cell details.

use std::collections::BTreeMap;

use gridcraft_core::{CellRef, RangeRef, Value};
use gridcraft_model::{PatternType, Sheet, Visibility, Workbook};
use serde_json::{Value as Json, json};

use super::{CommandSpec, EngineError, Result, Session, always, bad, bool_param, cell_param_on, cmd, has_doc, str_param, target_range, target_sheet};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(query "document.inspect", "Inspect Workbook", [], None, "{} → sheets, selection, names, tables, charts, dirty, trust (local|fromFile), history", has_doc, inspect),
        cmd!(query "sheet.read", "Read Range", [], None, "{range?: \"A1:D10\" (default: used range), sheet?, formulas?: bool, formatted?: bool} → rows of values", has_doc, read),
        cmd!(query "document.cells", "Read All Cells", [], None, "{} → styles, sheets: [{name, hidden, protected, merges, tables, hiddenRows, hiddenCols, filteredRows, cells: [{r, c, v, f?, s?, spill?}]}], names: every occupied cell, for a host keeping a read model", has_doc, all_cells),
        cmd!(query "cell.get", "Get Cell", [], None, "{cell?, sheet?} → value, formula, display text, style, comment, link, sources, python {code, output, typeName, preview, stale}", has_doc, cell_get),
        cmd!(query "selection.stats", "Selection Statistics", [], None, "{} → count, sum, average, min, max", has_doc, sel_stats),
        cmd!(query "history.list", "Undo History", [], None, "{} → undo/redo labels, grouping (an undo group is open)", has_doc, history),
        cmd!(query "app.commands", "List Commands", [], None, "{search?} → ids, labels, params", always, list_commands),
    ]
}

pub fn value_json(v: &Value) -> Json {
    match v {
        Value::Empty => Json::Null,
        Value::Number(n) => json!(n),
        Value::Text(t) => json!(t.as_ref()),
        Value::Bool(b) => json!(b),
        Value::Error(e) => json!({"error": e.as_str()}),
        Value::Array(a) => {
            let rows: Vec<Json> =
                (0..a.rows).map(|r| Json::Array((0..a.cols).map(|c| value_json(&a.get(r, c).cloned().unwrap_or_default())).collect())).collect();
            Json::Array(rows)
        }
    }
}

fn inspect(s: &mut Session, _: &Json) -> Result<Json> {
    let d = s.doc()?;
    let wb = &d.wb;
    let sheets: Vec<Json> = wb
        .sheets
        .iter()
        .map(|sh| {
            json!({
                "name": sh.name,
                "visibility": format!("{:?}", sh.visibility),
                "usedRange": sh.used_range().map(|r| r.a1()),
                "cells": sh.cells.len(),
                "freeze": sh.freeze,
                "merges": sh.merges.iter().map(|m| m.a1()).collect::<Vec<_>>(),
                "tables": sh.tables.iter().map(|t| json!({"name": t.name, "range": t.range.a1(), "style": t.style, "totals": t.totals_row, "columns": t.columns.iter().map(|c| c.name.clone()).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "charts": sh.charts.iter().map(|c| json!({"id": c.id, "kind": format!("{:?}", c.kind), "title": c.title, "at": c.anchor.cell.a1(), "series": c.series.len(), "dataTable": c.data_table.as_ref().map(|t| json!({"showKeys": t.show_keys, "horizontalBorder": t.horizontal_border, "verticalBorder": t.vertical_border, "outline": t.outline})), "trendlines": c.series.iter().flat_map(|s| s.trendlines.iter().map(|t| format!("{:?}", t.kind))).collect::<Vec<_>>()})).collect::<Vec<_>>(),
                "images": sh.images.len(),
                "shapes": sh.shapes.len(),
                "conditionalFormats": sh.cond_formats.len(),
                "validations": sh.validations.len(),
                "comments": sh.comments.len(),
                "autofilter": sh.autofilter.as_ref().map(|a| a.range.a1()),
                "protected": sh.protection.is_some(),
                "tabColor": sh.tab_color.and_then(|c| c.hex(&wb.theme)),
                "zoom": sh.zoom,
            })
        })
        .collect();
    Ok(json!({
        "title": d.display_title(),
        "path": d.path,
        "dirty": d.is_dirty(),
        "trust": d.trust,
        "activeSheet": wb.active_sheet(),
        "sheets": sheets,
        "selection": d.selection.a1(),
        "activeCell": d.selection.active.a1(),
        "names": wb.names.iter().map(|n| json!({"name": n.name, "refersTo": format!("={}", n.formula), "scope": n.scope})).collect::<Vec<_>>(),
        "calcMode": format!("{:?}", wb.calc.mode),
        "undo": d.undo.iter().rev().take(20).map(|e| e.label.clone()).collect::<Vec<_>>(),
        "redo": d.redo.iter().rev().take(20).map(|e| e.label.clone()).collect::<Vec<_>>(),
        "theme": wb.theme.name,
        "mode": format!("{:?}", s.mode),
    }))
}

fn read(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let d = s.doc()?;
    let sh = d.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    let r = match str_param(p, "range") {
        Some(_) => target_range(s, p)?,
        None => match sh.used_range() {
            Some(r) => r,
            None => return Ok(json!({"range": null, "values": []})),
        },
    };
    let r = match sh.used_range() {
        Some(u) if r.is_full_cols() || r.is_full_rows() => r.intersection(&u).unwrap_or(RangeRef::cell(r.start)),
        _ => r,
    };
    if r.count() > 1_000_000 {
        return Err(bad("sheet.read", "range larger than 1,000,000 cells; read it in parts"));
    }
    let formulas = bool_param(p, "formulas").unwrap_or(false);
    let formatted = bool_param(p, "formatted").unwrap_or(false);
    let mut rows = Vec::with_capacity(r.height() as usize);
    for row in r.start.row..=r.end.row {
        let mut cols = Vec::with_capacity(r.width() as usize);
        for col in r.start.col..=r.end.col {
            let c = CellRef::new(row, col);
            let v = if formulas && let Some(f) = sh.cell(c).and_then(|x| x.formula.as_ref()) {
                json!(format!("={}", f.text))
            } else if formatted {
                json!(crate::display::cell_text(&d.wb, sh, c))
            } else {
                value_json(&sh.value(c))
            };
            cols.push(v);
        }
        rows.push(Json::Array(cols));
    }
    Ok(json!({"range": r.a1(), "sheet": sh.name, "values": rows}))
}

fn cell_row(wb: &Workbook, sh: &Sheet, c: CellRef, styles: &mut BTreeMap<u32, Json>) -> Json {
    let mut row = json!({"r": c.row, "c": c.col, "v": value_json(&sh.value(c))});
    if let Some(f) = sh.cell(c).and_then(|x| x.formula.as_ref()) {
        row["f"] = json!(format!("={}", f.text));
    }
    if let Some(anchor) = sh.spill_anchor(c).or_else(|| sh.spill_ranges.contains_key(&c).then_some(c)) {
        row["spill"] = json!(anchor.a1());
    }
    let id = sh.style_id(c);
    if id.0 != 0 {
        row["s"] = json!(id.0);
        styles.entry(id.0).or_insert_with(|| {
            let st = wb.styles.get(id);
            json!({
                "numFmt": st.num_fmt.0.as_ref(),
                "bold": st.font.bold,
                "color": st.font.color.hex(&wb.theme),
                "fill": if st.fill.pattern == PatternType::None { None } else { st.fill.fg.hex(&wb.theme) },
            })
        });
    }
    row
}

fn all_cells(s: &mut Session, _: &Json) -> Result<Json> {
    let wb = &s.doc()?.wb;
    let mut styles = BTreeMap::new();
    let mut sheets = Vec::with_capacity(wb.sheets.len());
    for sh in &wb.sheets {
        let mut cells: Vec<Json> = sh.cells.iter().map(|(c, _)| cell_row(wb, sh, c, &mut styles)).collect();
        // What a formula spills holds no stored cell, and a write there would block the spill.
        for r in sh.spill_ranges.values() {
            for row in r.start.row..=r.end.row {
                for col in r.start.col..=r.end.col {
                    let c = CellRef::new(row, col);
                    if sh.cell(c).is_none() {
                        cells.push(cell_row(wb, sh, c, &mut styles));
                    }
                }
            }
        }
        sheets.push(json!({
            "name": sh.name,
            "hidden": !matches!(sh.visibility, Visibility::Visible),
            "protected": sh.protection.is_some(),
            "merges": sh.merges.iter().map(RangeRef::a1).collect::<Vec<_>>(),
            "tables": sh.tables.iter().map(|t| json!({"name": t.name, "range": t.range.a1()})).collect::<Vec<_>>(),
            "hiddenRows": sh.rows.iter().filter(|(_, i)| i.hidden).map(|(r, _)| *r).collect::<Vec<_>>(),
            "hiddenCols": sh.cols.iter().filter(|(_, i)| i.hidden).map(|(c, _)| *c).collect::<Vec<_>>(),
            "filteredRows": sh.rows.iter().filter(|(r, _)| sh.is_row_filtered(**r)).map(|(r, _)| *r).collect::<Vec<_>>(),
            "cells": cells,
        }));
    }
    Ok(json!({
        "styles": styles.into_iter().map(|(id, style)| (id.to_string(), style)).collect::<serde_json::Map<_, _>>(),
        "sheets": sheets,
        "names": wb.names.iter().map(|n| json!({"name": n.name, "refersTo": n.formula, "scope": n.scope.and_then(|i| wb.sheet(i)).map(|sh| sh.name.as_str()), "hidden": n.hidden})).collect::<Vec<_>>(),
    }))
}

fn cell_get(s: &mut Session, p: &Json) -> Result<Json> {
    let sheet = target_sheet(s, p)?;
    let d = s.doc()?;
    let sh = d.wb.sheet(sheet).ok_or(EngineError::NoDocument)?;
    let c = cell_param_on(s, p, "cell", sheet)?.unwrap_or(d.selection.active);
    let cell = sh.cell(c);
    let style = d.wb.styles.get(sh.style_id(c));
    Ok(json!({
        "cell": c.a1(),
        "value": value_json(&sh.value(c)),
        "type": if sh.cell_pictures.contains_key(&c) { "picture" } else { match sh.value(c) { Value::Empty => "empty", Value::Number(_) => "number", Value::Text(_) => "text", Value::Bool(_) => "boolean", Value::Error(_) => "error", Value::Array(_) => "array" } },
        "formula": cell.and_then(|x| x.formula.as_ref()).map(|f| format!("={}", f.text)),
        "text": crate::display::cell_text(&d.wb, sh, c),
        "input": sh.input_text(c),
        "picture": sh.cell_pictures.get(&c).map(|picture| json!({"mime": picture.mime, "alt": picture.alt, "bytes": picture.data.len()})),
        "spilledFrom": sh.spill_anchor(c).map(|a| a.a1()),
        "spill": sh.spill_ranges.get(&c).map(|r| r.a1()),
        "style": style,
        "merge": sh.merge_at(c).map(|m| m.a1()),
        "comment": sh.comments.get(&c),
        "hyperlink": sh.hyperlinks.get(&c),
        "sources": sh.sources.get(&c).map(|src| &src.links),
        "python": cell.and_then(|x| x.py.as_deref()).map(|py| json!({
            "code": py.code,
            "output": if py.object { "object" } else { "value" },
            "typeName": py.output.as_ref().map(|o| o.type_name.as_str()),
            "preview": py.output.as_ref().map(|o| o.preview.as_str()),
            "stale": py.stale,
        })),
    }))
}

fn sel_stats(s: &mut Session, _: &Json) -> Result<Json> {
    let d = s.doc()?;
    let sh = d.wb.active().ok_or(EngineError::NoDocument)?;
    let st = crate::display::stats(sh, &d.selection.ranges);
    serde_json::to_value(st).map_err(|e| EngineError::Other(e.to_string()))
}

fn history(s: &mut Session, _: &Json) -> Result<Json> {
    let d = s.doc()?;
    Ok(json!({
        "undo": d.undo.iter().rev().map(|e| e.label.clone()).collect::<Vec<_>>(),
        "redo": d.redo.iter().rev().map(|e| e.label.clone()).collect::<Vec<_>>(),
        "grouping": s.in_group(d.uid),
    }))
}

fn list_commands(s: &mut Session, p: &Json) -> Result<Json> {
    let q = str_param(p, "search").map(str::to_ascii_lowercase);
    let v: Vec<Json> = s
        .commands()
        .into_iter()
        .filter(|c| q.as_ref().is_none_or(|q| c.id.to_ascii_lowercase().contains(q.as_str()) || c.label.to_ascii_lowercase().contains(q.as_str())))
        .map(|c| serde_json::to_value(c).unwrap_or_default())
        .collect();
    Ok(Json::Array(v))
}

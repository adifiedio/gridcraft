//! The `postMessage` bridge to a same-origin parent page (`?host=parent`), for pages that embed
//! GridCraft in an `<iframe>` (such as a Nextcloud app). Messages are plain objects with a `type`:
//!
//! - `gridcraft:ready` (to the parent): `{ version }`, the app is listening; send a file now.
//! - `gridcraft:open` (from the parent): `{ name, bytes }`, bytes an `ArrayBuffer` or
//!   `Uint8Array`. It opens like a workbook picked with File › Open or dropped on the window.
//! - `gridcraft:save` (to the parent): `{ name, bytes }` (a transferred `ArrayBuffer`) for every
//!   file GridCraft would otherwise download: Save and Save As (an .xlsx workbook) and Export
//!   as PDF. The parent stores it and reports failures itself.
//! - `gridcraft:dirty` (to the parent): `{ dirty }` whenever unsaved changes appear or go away.
//! - `gridcraft:failed` (to the parent): `{ error }` when the app couldn't start.
//!
//! A parent that keeps the workbook itself, and follows every edit:
//!
//! - `gridcraft:follow` (from the parent): `{ name, bytes, sources }`, `sources` a JSON string as
//!   `sources.set` takes. Replaces whatever is open; nothing is left to undo.
//! - `gridcraft:followed` (to the parent): `{ error? }`, the answer to `gridcraft:follow`.
//! - `gridcraft:edit` (to the parent): `{ edit }`, a JSON string: one `LoggedCommand`, in order.
//! - `gridcraft:selection` (to the parent): `{ sheet, range, active, links }` when the selection
//!   moves. `links` is what the active cell cites.
//! - `gridcraft:goto` (from the parent): `{ sheet, cell }`.
//! - `gridcraft:editable` (from the parent): `{ editable }`, whether the user may change anything.
//!
//! Only the parent window on the page's own origin is heard or answered. In host mode the page
//! starts with an empty workbook instead of a sample, and the parent guards leaving the page.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gridcraft_ui_egui::{Inbox, Services, SheetApp};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;

/// Saving one of these hands the whole workbook to the parent, which stores it: the workbook
/// counts as saved under that name, as it would after Save on the desktop.
const WORKBOOK_EXTS: &[&str] = &["xlsx"];

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

#[derive(Clone)]
pub struct Bridge {
    window: web_sys::Window,
    parent: web_sys::Window,
    origin: String,
    /// Names of the files handed to the parent since the last frame.
    saved: Rc<RefCell<Vec<String>>>,
    /// The unsaved-changes state last reported to the parent.
    reported: Rc<Cell<Option<bool>>>,
    /// What the parent asked for since the last frame.
    asked: Rc<RefCell<Vec<Asked>>>,
    /// The selection last reported to the parent.
    selection: Rc<RefCell<Option<String>>>,
}

enum Asked {
    Follow { name: String, bytes: Vec<u8>, sources: serde_json::Value },
    Goto { sheet: String, cell: String },
    Editable(bool),
}

impl Bridge {
    /// `None` when the page isn't framed (there is no parent to talk to).
    pub fn new() -> Option<Self> {
        let window = web_sys::window()?;
        let parent = window.parent().ok().flatten()?;
        if js_sys::Object::is(&parent, &window) {
            return None;
        }
        let origin = window.location().origin().ok()?;
        Some(Self { window, parent, origin, saved: Rc::default(), reported: Rc::default(), asked: Rc::default(), selection: Rc::default() })
    }

    /// Send saved and exported files to the parent instead of downloading them, and hear files
    /// from it.
    pub fn connect(&self, services: &mut Services, inbox: Inbox, ctx: egui::Context) {
        self.listen(inbox, ctx);
        let bridge = self.clone();
        services.download = Some(Box::new(move |name: &str, bytes: &[u8]| {
            if let Err(e) = bridge.post_save(name, bytes) {
                log::error!("couldn't hand {name} to the page: {e}");
            }
        }));
    }

    /// `?author=`: the host's name for the user signs new notes and comments.
    pub fn set_author(app: &mut SheetApp, author: &str) {
        let author: String = author.trim().chars().take(MAX_AUTHOR_CHARS).collect();
        if !author.is_empty() {
            app.session.prefs.user_name = author;
        }
    }

    pub fn post_ready(&self) {
        self.send(&[("type", "gridcraft:ready".into()), ("version", env!("CARGO_PKG_VERSION").into())]);
    }

    pub fn post_failed(&self, error: &str) {
        self.send(&[("type", "gridcraft:failed".into()), ("error", error.into())]);
    }

    /// After each frame: a workbook handed to the parent this frame counts as saved under the
    /// name it was saved as, and the parent hears when unsaved changes appear or go away.
    pub fn sync(&self, app: &mut SheetApp) {
        self.answer(app);
        self.report_edits(app);
        let saved = std::mem::take(&mut *self.saved.borrow_mut());
        if let Some(name) = saved.iter().rev().find(|name| is_workbook(name))
            && let Some(doc) = app.session.active_mut()
        {
            doc.saved = doc.wb.clone();
            if doc.path.is_none() {
                doc.title = name.clone();
            }
        }
        let dirty = app.session.documents().iter().any(|d| d.is_dirty());
        if self.reported.get() != Some(dirty) && self.send(&[("type", "gridcraft:dirty".into()), ("dirty", dirty.into())]) {
            self.reported.set(Some(dirty));
        }
    }

    fn answer(&self, app: &mut SheetApp) {
        for asked in std::mem::take(&mut *self.asked.borrow_mut()) {
            match asked {
                Asked::Follow { name, bytes, sources } => {
                    let done = app.open_followed(&name, &bytes, sources);
                    let mut fields = vec![("type", "gridcraft:followed".into())];
                    if let Err(e) = &done {
                        fields.push(("error", e.as_str().into()));
                    }
                    self.send(&fields);
                    *self.selection.borrow_mut() = None;
                }
                Asked::Editable(editable) => app.session.read_only = !editable,
                Asked::Goto { sheet, cell } => {
                    if let Err(e) = app.go_to(&sheet, &cell) {
                        log::warn!("couldn't go to {sheet}!{cell}: {e}");
                    }
                }
            }
        }
    }

    /// A parent that follows hears each edit, then where the selection is. An edit that can't be
    /// sent is lost to the parent, whose next digest then differs; it loads its copy again.
    fn report_edits(&self, app: &mut SheetApp) {
        let Some(log) = app.session.edit_log.as_mut() else { return };
        for edit in std::mem::take(log) {
            match serde_json::to_string(&edit) {
                Ok(json) => {
                    self.send(&[("type", "gridcraft:edit".into()), ("edit", json.as_str().into())]);
                }
                Err(e) => log::error!("couldn't report an edit: {e}"),
            }
        }
        let Some(d) = app.session.active() else { return };
        let (sheet, links) = d.wb.active().map_or((String::new(), Vec::new()), |sh| {
            (sh.name.clone(), sh.sources.get(&d.selection.active).map(|s| s.links.clone()).unwrap_or_default())
        });
        let range = d.selection.a1();
        let active = d.selection.active.a1();
        let now = format!("{sheet}\u{0}{range}\u{0}{active}");
        if self.selection.borrow().as_deref() == Some(now.as_str()) {
            return;
        }
        let links = serde_json::to_string(&links).unwrap_or_else(|_| "[]".to_string());
        let fields = [
            ("type", "gridcraft:selection".into()),
            ("sheet", sheet.as_str().into()),
            ("range", range.as_str().into()),
            ("active", active.as_str().into()),
            ("links", links.as_str().into()),
        ];
        if self.send(&fields) {
            *self.selection.borrow_mut() = Some(now);
        }
    }

    fn post_save(&self, path: &str, bytes: &[u8]) -> Result<(), String> {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string());
        let buffer = js_sys::Uint8Array::from(bytes).buffer();
        let message = message(&[("type", "gridcraft:save".into()), ("name", name.as_str().into()), ("bytes", buffer.clone().into())])
            .ok_or_else(|| "couldn't build the message".to_string())?;
        self.parent
            .post_message_with_transfer(&message, &self.origin, &js_sys::Array::of1(&buffer))
            .map_err(|e| e.as_string().unwrap_or_else(|| format!("{e:?}")))?;
        self.saved.borrow_mut().push(name);
        Ok(())
    }

    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
        let parent = self.parent.clone();
        let origin = self.origin.clone();
        let asked = self.asked.clone();
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |event: web_sys::MessageEvent| {
            if event.origin() != origin || !event.source().is_some_and(|s| js_sys::Object::is(&s, &parent)) {
                return;
            }
            let data = event.data();
            match string(&data, "type").as_deref() {
                Some("gridcraft:open") => {}
                Some("gridcraft:follow") => {
                    let sources = string(&data, "sources").and_then(|s| serde_json::from_str(&s).ok());
                    if let (Some(name), Some(bytes), Some(sources)) = (string(&data, "name"), bytes(&data), sources) {
                        asked.borrow_mut().push(Asked::Follow { name, bytes, sources });
                        ctx.request_repaint();
                    }
                    return;
                }
                Some("gridcraft:editable") => {
                    let editable = js_sys::Reflect::get(&data, &"editable".into()).ok().and_then(|v| v.as_bool());
                    asked.borrow_mut().push(Asked::Editable(editable.unwrap_or(false)));
                    ctx.request_repaint();
                    return;
                }
                Some("gridcraft:goto") => {
                    if let (Some(sheet), Some(cell)) = (string(&data, "sheet"), string(&data, "cell")) {
                        asked.borrow_mut().push(Asked::Goto { sheet, cell });
                        ctx.request_repaint();
                    }
                    return;
                }
                _ => return,
            }
            let name = string(&data, "name").filter(|n| !n.trim().is_empty()).unwrap_or_else(|| "Book.xlsx".to_string());
            let Some(bytes) = bytes(&data) else { return };
            inbox.lock().unwrap_or_else(|e| e.into_inner()).push((name, bytes));
            ctx.request_repaint();
        });
        if self.window.add_event_listener_with_callback("message", on_message.as_ref().unchecked_ref()).is_ok() {
            // The listener lives as long as the page.
            on_message.forget();
        }
    }

    /// Post a message to the parent; false if it couldn't be sent.
    fn send(&self, fields: &[(&str, JsValue)]) -> bool {
        message(fields).is_some_and(|m| self.parent.post_message(&m, &self.origin).is_ok())
    }
}

fn is_workbook(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| WORKBOOK_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

fn message(fields: &[(&str, JsValue)]) -> Option<js_sys::Object> {
    let m = js_sys::Object::new();
    for (key, value) in fields {
        js_sys::Reflect::set(&m, &JsValue::from_str(key), value).ok()?;
    }
    Some(m)
}

fn string(data: &JsValue, key: &str) -> Option<String> {
    js_sys::Reflect::get(data, &JsValue::from_str(key)).ok()?.as_string()
}

/// `bytes` as an `ArrayBuffer` or `Uint8Array`.
fn bytes(data: &JsValue) -> Option<Vec<u8>> {
    let bytes = js_sys::Reflect::get(data, &JsValue::from_str("bytes")).ok()?;
    if let Some(array) = bytes.dyn_ref::<js_sys::Uint8Array>() {
        Some(array.to_vec())
    } else if bytes.is_instance_of::<js_sys::ArrayBuffer>() {
        Some(js_sys::Uint8Array::new(&bytes).to_vec())
    } else {
        None
    }
}

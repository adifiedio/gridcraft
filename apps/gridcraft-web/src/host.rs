//! Host modes: a page that runs GridCraft (as an iframe, or in its own document) opens workbooks
//! in it and stores what it saves instead of the browser downloading. The messages and the two
//! transports are `craft-web-host`'s (storytold/craft-libs), the same contract as the other Craft
//! apps' bridges, with the `gridcraft:` prefix. This module is GridCraft's part of it.
//!
//! - `?host=parent` (an `<iframe>`, such as a Nextcloud app) talks to the same-origin parent over
//!   `postMessage`; `data-host="page"` on the canvas (the page's own document, no iframe) uses
//!   DOM `CustomEvent`s on the canvas instead. `?host=parent` wins when both are present. With no
//!   parent, or a parent on another origin, there is no bridge: the normal app with downloads.
//! - `gridcraft:ready` once the app is listening; send a file now. GridCraft reads bytes, so it
//!   says `file: false` and a host sends `bytes`.
//! - `gridcraft:open` (from the host): `{ name, bytes }`. It opens like a workbook picked with
//!   File › Open or dropped on the window; a file over [`MAX_OPEN_BYTES`] is refused before it is
//!   copied.
//! - `gridcraft:save` (to the host): `{ name, saveAs, bytes }` for every file GridCraft would
//!   otherwise download: Save and Save As (an .xlsx workbook) and Export as PDF. The host stores
//!   it and reports failures itself. GridCraft doesn't tell the host which saves make a file the
//!   document, so `saveAs` is always `false`.
//! - `gridcraft:dirty` whenever unsaved changes appear or go away, `gridcraft:failed` when the
//!   app couldn't start.
//!
<<<<<<< HEAD
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
=======
//! In both host modes the page starts with an empty workbook instead of a sample, saves and PDF
//! exports are handed to the page rather than downloaded, and `?author=` names the user for new
//! notes and comments.
>>>>>>> gridcraft-upstream/main

use std::cell::RefCell;
use std::rc::Rc;

use craft_web_host::web::{FileData, Host, Message};
use gridcraft_ui_egui::{Inbox, Services, SheetApp};

use crate::hostmode::open_name;

/// The app id in the host messages (`gridcraft:ready`).
const APP: &str = "gridcraft";

/// Saved files that count as a workbook: the document is then clean under the name it was saved as.
const WORKBOOK_EXTS: &[&str] = &["xlsx"];

/// The largest workbook taken from the host page, refused before it is copied.
const MAX_OPEN_BYTES: u64 = 512 << 20;

/// The longest user name taken from `?author=`.
const MAX_AUTHOR_CHARS: usize = 200;

#[derive(Clone)]
pub struct Bridge {
    host: Rc<Host>,
    /// Names of the files handed to the host since the last frame.
    saved: Rc<RefCell<Vec<String>>>,
<<<<<<< HEAD
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
=======
}

impl Bridge {
    /// The bridge for this page, or `None` when GridCraft isn't hosted: no `?host=parent` and no
    /// `data-host="page"` on the canvas, an unframed `?host=parent` page (it stays the normal app
    /// with downloads), or a parent on another origin.
    pub fn detect(canvas: &web_sys::HtmlCanvasElement) -> Option<Self> {
        Some(Self { host: Rc::new(Host::connect(APP, canvas)?), saved: Rc::default() })
>>>>>>> gridcraft-upstream/main
    }

    /// Send saved and exported files to the host instead of downloading them, and hear files
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
        let _ = self.host.post_ready(env!("CARGO_PKG_VERSION"), false);
    }

    pub fn post_failed(&self, error: &str) {
        let _ = self.host.post_failed(error, None);
    }

    /// After each frame: a workbook handed to the host this frame counts as saved under the
    /// name it was saved as, and the host hears when unsaved changes appear or go away (the
    /// crate does not repeat a state it already reported).
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
        self.host.post_dirty(app.session.documents().iter().any(|d| d.is_dirty()));
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
        let name = file_name(path);
        self.host.post_save(&name, bytes, None, false)?;
        self.saved.borrow_mut().push(name);
        Ok(())
    }

    fn listen(&self, inbox: Inbox, ctx: egui::Context) {
<<<<<<< HEAD
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
=======
        self.host.listen(move |message| {
            if let Message::Open { name, data } = message {
                deliver_open(&inbox, &ctx, &name, &data);
            }
>>>>>>> gridcraft-upstream/main
        });
    }
}

/// The file an open message carries, pushed onto the inbox for the next frame, like a dropped
/// file. A host's file is untrusted input: an oversized one is refused before it is copied into
/// the app's memory (the data's own length, not the host's claim).
fn deliver_open(inbox: &Inbox, ctx: &egui::Context, name: &str, data: &FileData) {
    let FileData::Bytes(array) = data else {
        log::warn!("host file {name} came as a File; GridCraft reads bytes only; not opened");
        return;
    };
    if data.size() > MAX_OPEN_BYTES as f64 {
        log::warn!("host file is {} bytes, over the {} MiB host open limit; not opened", data.size(), MAX_OPEN_BYTES >> 20);
        return;
    }
    inbox.lock().unwrap_or_else(|e| e.into_inner()).push((open_name(name), array.to_vec()));
    ctx.request_repaint();
}

fn is_workbook(name: &str) -> bool {
    name.rsplit_once('.').is_some_and(|(_, ext)| WORKBOOK_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)))
}

fn file_name(path: &str) -> String {
    std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| path.to_string())
}

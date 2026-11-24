//! The client: when a chest joined to others is open, a panel beside the chest's own screen
//! shows everything in all of them as one list. The chest screen, its slots, the inventory and
//! the cursor stay the game's own; the panel is drawn in the game's look (its panel, slot
//! frames, the creative search field and scrollbar, item counts at half size as storage mods
//! show big counts). Every click on it is a request the server answers with a new view.
use crate::entries::{self, Shown, Sort};
use crate::payloads::{self, Click, Comfort, View, comfort};
use mod_api::client::hud_layout::{Blend, PoseOp};
use mod_api::client::widgets::{Outcome, TextField, WidgetId, Widgets};
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{ClientHost, DrawContext, GuiDraw, Identifier, ScreenView, Text, VarInt};
use std::sync::Mutex;

type Rect = [f32; 4];

const SLOT: f32 = 18.0;
const MAX_COLUMNS: usize = 9;
const MIN_COLUMNS: usize = 3;
/// The panel's inner margin, the search row above the list and the button row under it.
const MARGIN: f32 = 7.0;
const TOP: f32 = 18.0;
const BOTTOM: f32 = 26.0;
const SCROLL_W: f32 = 14.0;
const BUTTON: f32 = 18.0;
/// The gap between the chest's screen and the panel.
const GAP: f32 = 2.0;

/// Where the panel sits this frame, beside the chest's screen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Panel {
    rect: Rect,
    columns: usize,
    rows: usize,
}

impl Panel {
    /// Beside `chest` on a `size` screen: to its right (the game's recipe book opens on the left),
    /// else to its left, as wide as fits (3 to 9 columns), as tall as the chest's screen.
    fn beside(chest: Rect, size: [f32; 2]) -> Option<Self> {
        let rows = ((chest[3] - TOP - BOTTOM) / SLOT).floor().max(1.0) as usize;
        let fits = |room: f32| (((room - 2.0 * MARGIN - SCROLL_W - 2.0) / SLOT).floor().max(0.0) as usize).min(MAX_COLUMNS);
        let width = |columns: usize| 2.0 * MARGIN + columns as f32 * SLOT + 2.0 + SCROLL_W;
        let left = fits(chest[0] - GAP);
        let right = fits(size[0] - (chest[0] + chest[2]) - GAP);
        let (columns, x) = if right >= MIN_COLUMNS {
            (right, chest[0] + chest[2] + GAP)
        } else if left >= MIN_COLUMNS {
            (left, chest[0] - GAP - width(left))
        } else {
            return None;
        };
        Some(Panel { rect: [x, chest[1], width(columns), chest[3]], columns, rows })
    }

    fn grid(&self) -> [f32; 2] {
        [self.rect[0] + MARGIN, self.rect[1] + TOP]
    }

    fn search(&self) -> Rect {
        [self.rect[0] + MARGIN, self.rect[1] + 3.0, self.columns as f32 * SLOT, 12.0]
    }

    fn track(&self) -> Rect {
        let [gx, gy] = self.grid();
        [gx + self.columns as f32 * SLOT + 2.0, gy, SCROLL_W, self.rows as f32 * SLOT]
    }

    fn buttons_y(&self) -> f32 {
        self.grid()[1] + self.rows as f32 * SLOT + 2.0
    }

    fn sort(&self) -> Rect {
        let [gx, _] = self.grid();
        [gx + self.columns as f32 * SLOT - BUTTON, self.buttons_y(), BUTTON, BUTTON]
    }

    fn empty(&self) -> Rect {
        let s = self.sort();
        [s[0] - BUTTON, s[1], BUTTON, BUTTON]
    }

    /// The list position (in shown order, from the first row shown) under `mouse`.
    fn slot_at(&self, mouse: [f32; 2]) -> Option<usize> {
        let [gx, gy] = self.grid();
        let (x, y) = (mouse[0] - gx - 1.0, mouse[1] - gy - 1.0);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (col, row) = ((x / SLOT) as usize, (y / SLOT) as usize);
        (col < self.columns && row < self.rows && x % SLOT < 16.0 && y % SLOT < 16.0).then_some(row * self.columns + col)
    }
}

/// What the panel knows: the server's view, the player's choices, the search field.
#[derive(Default)]
struct Ui {
    view: Option<View>,
    /// Names in the player's language for the current view, made once per view.
    names: Vec<String>,
    /// Entry indices (into the view) in the order shown, after search and sort.
    shown: Vec<usize>,
    sort: Sort,
    row: usize,
    panel: Option<Panel>,
    widgets: Option<(Widgets, WidgetId)>,
    query: String,
    /// Requests waiting for the next tick (sending needs the mutable host).
    outbox: Vec<Request>,
}

enum Request {
    Click(Click),
    Comfort(Comfort),
}

static UI: Mutex<Option<Ui>> = Mutex::new(None);

fn ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    let mut guard = UI.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Ui::default))
}

/// The server sent a view: keep it; names and order are made on the next frame.
pub fn viewed(view: View) {
    ui(|u| {
        u.view = Some(view);
        u.names.clear();
    });
}

/// The chest closed: forget its view (search and sort stay for the next chest).
pub fn closed() {
    ui(|u| {
        u.view = None;
        u.panel = None;
        u.widgets = None;
        u.row = 0;
    });
}

pub fn tick(host: &mut dyn ClientHost) {
    for request in ui(|u| std::mem::take(&mut u.outbox)) {
        let sent = match request {
            Request::Click(r) => payloads::CLICK.get().map(|t| t.send(host, &r)),
            Request::Comfort(r) => payloads::COMFORT.get().map(|t| t.send(host, &r)),
        };
        if let Some(Err(e)) = sent {
            eprintln!("[storage] sending a request: {e:?}");
        }
    }
}

fn item_name(host: &dyn ClientHost, id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    host.translate(&format!("item.{ns}.{path}"))
        .or_else(|| host.translate(&format!("block.{ns}.{path}")))
        .unwrap_or_else(|| id.to_owned())
}

fn tr(host: &dyn ClientHost, key: &str, fallback: &str) -> String {
    host.translate(key).unwrap_or_else(|| fallback.to_owned())
}

fn total(view: &View, i: usize) -> u64 {
    view.totals.get(i).map_or(0, |t| t.0.max(0) as u64)
}

/// Names, search and sort for the current view (after a new view, a keystroke or a sort change).
fn reorder(host: &dyn ClientHost, u: &mut Ui) {
    let Some(view) = &u.view else { return };
    if u.names.len() != view.ids.len() {
        u.names = view.ids.iter().map(|id| item_name(host, id)).collect();
    }
    let all: Vec<Shown> =
        view.ids.iter().enumerate().map(|(i, id)| Shown { name: &u.names[i], id, total: total(view, i) }).collect();
    let has_tag = |id: &str, tag: &str| {
        host.registry_entry("minecraft:item", id).and_then(|e| host.registry_entry_in_tag(&e, tag)).unwrap_or(false)
    };
    let query = u.query.clone();
    u.shown = entries::order(&all, u.sort)
        .into_iter()
        .filter(|&i| entries::matches(&query, &all[i], |t| has_tag(all[i].id, t)))
        .collect();
    let columns = u.panel.map_or(MAX_COLUMNS, |p| p.columns);
    let rows = u.panel.map_or(1, |p| p.rows);
    u.row = u.row.min(u.shown.len().div_ceil(columns).saturating_sub(rows));
}

fn inside([x, y, w, h]: Rect, [mx, my]: [f32; 2]) -> bool {
    mx >= x && my >= y && mx < x + w && my < y + h
}

/// The game's panel look: light grey, a white top-left bevel, a dark bottom-right one, a black
/// outline with cut corners.
fn panel_frame(draw: &mut dyn DrawContext, [x, y, w, h]: Rect) {
    draw.fill([x + 1.0, y, w - 2.0, 1.0], 0xFF00_0000);
    draw.fill([x + 1.0, y + h - 1.0, w - 2.0, 1.0], 0xFF00_0000);
    draw.fill([x, y + 1.0, 1.0, h - 2.0], 0xFF00_0000);
    draw.fill([x + w - 1.0, y + 1.0, 1.0, h - 2.0], 0xFF00_0000);
    draw.fill([x + 1.0, y + 1.0, w - 2.0, h - 2.0], 0xFFC6_C6C6);
    draw.fill([x + 1.0, y + 1.0, w - 3.0, 2.0], 0xFFFF_FFFF);
    draw.fill([x + 1.0, y + 1.0, 2.0, h - 3.0], 0xFFFF_FFFF);
    draw.fill([x + 3.0, y + h - 3.0, w - 4.0, 2.0], 0xFF55_5555);
    draw.fill([x + w - 3.0, y + 3.0, 2.0, h - 4.0], 0xFF55_5555);
}

/// A slot frame as the game draws one: dark top-left, white bottom-right, grey inside.
fn frame(draw: &mut dyn DrawContext, [x, y]: [f32; 2]) {
    draw.fill([x, y, 18.0, 1.0], 0xFF37_3737);
    draw.fill([x, y, 1.0, 18.0], 0xFF37_3737);
    draw.fill([x + 1.0, y + 17.0, 17.0, 1.0], 0xFFFF_FFFF);
    draw.fill([x + 17.0, y + 1.0, 1.0, 17.0], 0xFFFF_FFFF);
    draw.fill([x + 1.0, y + 1.0, 16.0, 16.0], 0xFF8B_8B8B);
}

/// An item count at half size in the slot's bottom-right corner.
fn count(draw: &mut dyn DrawContext, n: u64, [x, y]: [f32; 2]) {
    let text = entries::count_text(n);
    let w = draw.text_width(&text) * 0.5;
    let [tx, ty] = [(x + 17.0 - w) * 2.0, (y + 12.0) * 2.0];
    let _ = draw.gui_paint(&[
        GuiDraw::Push(vec![PoseOp::Scale(0.5, 0.5)]),
        GuiDraw::Text { text: Text::literal(text), x: tx as i32, y: ty as i32, color: 0xFFFF_FFFF, shadow: true },
        GuiDraw::Pop,
    ]);
}

fn sprite(draw: &mut dyn DrawContext, name: &str, [x, y, w, h]: Rect) {
    let _ = draw.gui_paint(&[GuiDraw::Sprite {
        sprite: name.to_owned().into(),
        x: x as i32,
        y: y as i32,
        w: w as i32,
        h: h as i32,
        region: None,
        color: 0xFFFF_FFFF,
        blend: Blend::Textured,
    }]);
}

fn icon(host: &dyn ClientHost, draw: &mut dyn DrawContext, id: &str, at: [f32; 2]) {
    if let Some(key) = Identifier::parse(id).ok().and_then(|k| host.item_key(&k)) {
        draw.item(key, at, 16.0);
    }
}

/// The sort button's icon: the order it is in.
fn sort_icon(sort: Sort) -> &'static str {
    match sort {
        Sort::Count => "minecraft:bundle",
        Sort::Name => "minecraft:name_tag",
        Sort::Mod => "minecraft:book",
    }
}

/// A button in the list's own slot look (one grammar for the panel): the slot frame, the icon,
/// the game's white hover.
fn button(host: &dyn ClientHost, draw: &mut dyn DrawContext, rect: Rect, id: &str, hovered: bool) {
    frame(draw, [rect[0], rect[1]]);
    icon(host, draw, id, [rect[0] + 1.0, rect[1] + 1.0]);
    if hovered {
        draw.fill([rect[0] + 1.0, rect[1] + 1.0, 16.0, 16.0], 0x80FF_FFFF);
    }
}

/// Whether the panel shows: a container screen is open and its chest is joined to others.
fn showing(screen: &ScreenView, u: &Ui) -> bool {
    screen.menu.is_some() && u.view.as_ref().is_some_and(|v| v.containers.0 > 1)
}

/// Draws the panel under the screen's own items, tooltips and cursor stack.
pub fn draw(host: &dyn ClientHost, draw: &mut dyn DrawContext, screen: &ScreenView) {
    ui(|u| {
        if !showing(screen, u) {
            return;
        }
        let Some(panel) = Panel::beside(screen.rect, draw.size()) else { return };
        if u.panel != Some(panel) || u.names.is_empty() {
            u.panel = Some(panel);
            reorder(host, u);
        }
        let Some(view) = u.view.clone() else { return };
        panel_frame(draw, panel.rect);
        // The search field, made once per chest opened, showing the search kept from the last one
        // (the list is filtered by it, so it is always in sight).
        let field = panel.search();
        let query = u.query.clone();
        let (widgets, id) = u.widgets.get_or_insert_with(|| {
            let mut w = Widgets::new();
            let mut text = TextField::new(field, 50);
            text.set_value(&query);
            let id = w.add_text_field(text);
            (w, id)
        });
        if let Some(f) = widgets.text_field_mut(*id) {
            f.rect = field;
            f.bordered = true;
            f.suggestion = f.value().is_empty().then(|| tr(host, "storage.search", "Search…"));
        }
        let _ = widgets.draw(draw, host, screen.mouse);
        let [gx, gy] = panel.grid();
        for r in 0..panel.rows {
            for c in 0..panel.columns {
                let at = [gx + c as f32 * SLOT, gy + r as f32 * SLOT];
                frame(draw, at);
                let Some(&i) = u.shown.get((u.row + r) * panel.columns + c) else { continue };
                icon(host, draw, &view.ids[i], [at[0] + 1.0, at[1] + 1.0]);
                if total(&view, i) > 1 {
                    count(draw, total(&view, i), at);
                }
            }
        }
        // The scrollbar: the creative inventory's knob, greyed when everything fits.
        let rows = u.shown.len().div_ceil(panel.columns);
        let track = panel.track();
        draw.fill(track, 0xFF8B_8B8B);
        let (knob, share) = if rows > panel.rows {
            ("container/creative_inventory/scroller", u.row as f32 / (rows - panel.rows) as f32)
        } else {
            ("container/creative_inventory/scroller_disabled", 0.0)
        };
        sprite(draw, knob, [track[0] + 1.0, track[1] + share * (track[3] - 15.0), 12.0, 15.0]);
        // How many containers are joined, then the buttons.
        let joined = tr(host, "storage.joined", "%s chests").replace("%s", &view.containers.0.to_string());
        draw.text(&joined, [panel.rect[0] + MARGIN, panel.buttons_y() + 5.0], 0xFF40_4040, false);
        button(host, draw, panel.sort(), sort_icon(u.sort), inside(panel.sort(), screen.mouse));
        if view.comforts.0 & 2 != 0 {
            button(host, draw, panel.empty(), "minecraft:hopper", inside(panel.empty(), screen.mouse));
        }
    });
}

/// Hover: the list slot's highlight and tooltip, and the buttons' tooltips, over everything.
pub fn hover(host: &dyn ClientHost, draw: &mut dyn DrawContext, screen: &ScreenView) {
    ui(|u| {
        let (Some(panel), Some(view)) = (u.panel, &u.view) else { return };
        if !showing(screen, u) {
            return;
        }
        let mouse = screen.mouse;
        if let Some(pos) = panel.slot_at(mouse) {
            let [gx, gy] = panel.grid();
            let at = [gx + 1.0 + (pos % panel.columns) as f32 * SLOT, gy + 1.0 + (pos / panel.columns) as f32 * SLOT];
            draw.fill([at[0], at[1], 16.0, 16.0], 0x80FF_FFFF);
            if let Some(&i) = u.shown.get(u.row * panel.columns + pos) {
                let mut lines = vec![Text::literal(u.names[i].clone()), Text::literal(total(view, i).to_string())];
                if view.comforts.0 & 1 != 0 {
                    lines.push(Text::literal(tr(host, "storage.find_hint", "F: show the chest")));
                }
                let _ = draw.gui_tooltip(&lines, mouse);
            }
        } else if inside(panel.sort(), mouse) {
            let key = match u.sort {
                Sort::Count => "storage.sort.count",
                Sort::Name => "storage.sort.name",
                Sort::Mod => "storage.sort.mod",
            };
            let _ = draw.gui_tooltip(&[Text::literal(tr(host, key, key))], mouse);
        } else if view.comforts.0 & 2 != 0 && inside(panel.empty(), mouse) {
            let _ = draw.gui_tooltip(&[Text::literal(tr(host, "storage.empty", "Put everything in"))], mouse);
        }
    });
}

/// A press on the panel: the list, the search field or a button. Presses elsewhere are the
/// chest screen's own.
pub fn press(host: &dyn ClientHost, screen: &ScreenView, button: i32, modifiers: u32) -> bool {
    let mouse = screen.mouse;
    let shift = modifiers & 1 != 0;
    ui(|u| {
        let Some(panel) = u.panel else { return false };
        if !showing(screen, u) || !inside(panel.rect, mouse) {
            if let Some((widgets, _)) = &mut u.widgets {
                widgets.set_focus(host, None);
            }
            return false;
        }
        if let Some((widgets, _)) = &mut u.widgets
            && !matches!(widgets.mouse_pressed(host, mouse, button, modifiers), Outcome::Ignored)
        {
            return true;
        }
        let Some(view) = &u.view else { return true };
        if let Some(pos) = panel.slot_at(mouse) {
            let index = u.shown.get(u.row * panel.columns + pos).copied();
            let id = index.map(|i| view.ids[i].clone()).unwrap_or_default();
            let index = index.map_or(-1, |i| i as i32);
            u.outbox.push(Request::Click(Click { index: VarInt(index), id, button: VarInt(button), shift }));
        } else if inside(panel.sort(), mouse) {
            u.sort = u.sort.next();
            reorder(host, u);
        } else if view.comforts.0 & 2 != 0 && inside(panel.empty(), mouse) {
            u.outbox.push(Request::Comfort(Comfort { kind: VarInt(comfort::EMPTY), index: VarInt(-1), id: String::new() }));
        }
        true
    })
}

pub fn scroll(screen: &ScreenView, by: f64) -> bool {
    ui(|u| {
        let Some(panel) = u.panel else { return false };
        if !showing(screen, u) || !inside(panel.rect, screen.mouse) {
            return false;
        }
        let max = u.shown.len().div_ceil(panel.columns).saturating_sub(panel.rows);
        u.row = if by > 0.0 { u.row.saturating_sub(1) } else { (u.row + 1).min(max) };
        true
    })
}

/// Keys while a chest is open: typing goes to the search field when it has the focus (so `E`
/// doesn't close the chest); `F` over a list slot finds it.
pub fn key(host: &dyn ClientHost, screen: &ScreenView, key: &str, modifiers: u32) -> bool {
    if key == "key.keyboard.escape" {
        return false;
    }
    ui(|u| {
        let Some(panel) = u.panel else { return false };
        if !showing(screen, u) {
            return false;
        }
        if let Some((widgets, id)) = &mut u.widgets
            && widgets.focused() == Some(*id)
        {
            if let Outcome::Changed(_) = widgets.key_pressed(host, key, modifiers) {
                u.query = widgets.text_field(*id).map(|f| f.value().to_owned()).unwrap_or_default();
                u.row = 0;
                reorder(host, u);
            }
            return true;
        }
        let Some(view) = &u.view else { return false };
        if key == "key.keyboard.f"
            && view.comforts.0 & 1 != 0
            && let Some(pos) = panel.slot_at(screen.mouse)
            && let Some(&i) = u.shown.get(u.row * panel.columns + pos)
        {
            let id = view.ids[i].clone();
            u.outbox.push(Request::Comfort(Comfort { kind: VarInt(comfort::FIND), index: VarInt(i as i32), id }));
            return true;
        }
        false
    })
}

pub fn char_typed(host: &dyn ClientHost, screen: &ScreenView, c: char) -> bool {
    ui(|u| {
        if !showing(screen, u) {
            return false;
        }
        let Some((widgets, id)) = &mut u.widgets else { return false };
        if widgets.focused() != Some(*id) {
            return false;
        }
        if let Outcome::Changed(_) = widgets.char_typed(host, c) {
            u.query = widgets.text_field(*id).map(|f| f.value().to_owned()).unwrap_or_default();
            u.row = 0;
            reorder(host, u);
        }
        true
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single chest's screen (176 × 166) centred on a screen `w` wide.
    fn chest(w: f32) -> Rect {
        [((w - 176.0) / 2.0).floor(), 60.0, 176.0, 166.0]
    }

    #[test]
    fn the_panel_sits_beside_the_chest_and_never_off_screen() {
        for w in [320.0, 400.0, 427.0, 480.0, 533.0, 640.0, 960.0] {
            let Some(p) = Panel::beside(chest(w), [w, 300.0]) else {
                assert!(w < 400.0, "no room at {w}");
                continue;
            };
            assert!(p.rect[0] >= 0.0 && p.rect[0] + p.rect[2] <= w, "{w}: {:?}", p.rect);
            let c = chest(w);
            assert!(p.rect[0] + p.rect[2] <= c[0] || p.rect[0] >= c[0] + c[2], "{w}: overlaps the chest");
            assert!((MIN_COLUMNS..=MAX_COLUMNS).contains(&p.columns));
            assert_eq!(p.rows, 6);
            assert!(p.buttons_y() + BUTTON <= p.rect[1] + p.rect[3]);
        }
    }

    #[test]
    fn a_big_chest_gets_more_rows() {
        let p = Panel::beside([200.0, 20.0, 176.0, 222.0], [576.0, 270.0]).unwrap();
        assert_eq!(p.rows, 9);
    }
}

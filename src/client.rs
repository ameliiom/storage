//! The client: when a chest (or a station) joined to others is open, a panel beside the game's own
//! screen shows everything in all of them as one list, with search, category tabs, sort orders,
//! a capacity bar and the organizing buttons. The chest screen, its slots, the inventory and the
//! cursor stay the game's own; the panel is drawn in the game's look (its panel, slot frames, the
//! creative search field and scrollbar, item counts at half size as storage mods show big counts).
//! Every click on it is a request the server answers with a new view.
use crate::entries::{self, Shown, Sort};
use crate::organize::Category;
use crate::payloads::{self, Click, Comfort, View, comfort};
use mod_api::client::hud_layout::{Blend, PoseOp};
use mod_api::client::widgets::{Outcome, TextField, WidgetId, Widgets};
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{ClientHost, DrawContext, GuiDraw, Identifier, ScreenView, Text, VarInt};
use std::collections::HashMap;
use std::sync::Mutex;

type Rect = [f32; 4];

const SLOT: f32 = 18.0;
const MAX_COLUMNS: usize = 9;
const MIN_COLUMNS: usize = 3;
/// The panel's inner margin and the search row above the list.
const MARGIN: f32 = 7.0;
const TOP: f32 = 18.0;
const SCROLL_W: f32 = 14.0;
/// The capacity bar under the list, and the gap around it.
const BAR_H: f32 = 4.0;
const BAR_GAP: f32 = 3.0;
/// The gap between the chest's screen and the panel.
const GAP: f32 = 2.0;

/// A tab of the strip at the panel's side: everything, one category, or other mods' items.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tab {
    All,
    Of(Category),
    Mods,
}

/// The tabs, top to bottom, each with the vanilla item drawn on it and its tooltip key.
const TABS: [(Tab, &str, &str); 7] = [
    (Tab::All, "minecraft:chest", "storage.tab.all"),
    (Tab::Of(Category::Gear), "minecraft:iron_sword", "storage.tab.gear"),
    (Tab::Of(Category::Food), "minecraft:bread", "storage.tab.food"),
    (Tab::Of(Category::Redstone), "minecraft:redstone", "storage.tab.redstone"),
    (Tab::Of(Category::Materials), "minecraft:iron_ingot", "storage.tab.materials"),
    (Tab::Of(Category::Blocks), "minecraft:grass_block", "storage.tab.blocks"),
    (Tab::Mods, "minecraft:knowledge_book", "storage.tab.mods"),
];

/// The buttons under the list, in order: what each asks, its icon and its tooltip key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Button {
    /// Sort chests (compact and order the chests themselves).
    Tidy,
    /// Put in what storage already holds.
    Match,
    /// Fill partial stacks from storage.
    Restock,
    /// Put the whole inventory in (a comfort).
    Empty,
    /// The list's order (right-click: direction).
    Order,
}

const BUTTONS: [Button; 5] = [Button::Tidy, Button::Match, Button::Restock, Button::Empty, Button::Order];

/// Where the panel sits this frame, beside the chest's screen.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Panel {
    rect: Rect,
    columns: usize,
    rows: usize,
    /// Rows of buttons under the list (the buttons wrap when the panel is narrow).
    button_rows: usize,
}

impl Panel {
    /// Beside `chest` on a `size` screen: to its right (the game's recipe book opens on the left),
    /// else to its left, as wide as fits (3 to 9 columns), as tall as the chest's screen.
    fn beside(chest: Rect, size: [f32; 2]) -> Option<Self> {
        let fixed = 2.0 * MARGIN + 2.0 + SCROLL_W + 2.0 + SLOT;
        let fits = |room: f32| (((room - fixed) / SLOT).floor().max(0.0) as usize).min(MAX_COLUMNS);
        let width = |columns: usize| fixed + columns as f32 * SLOT;
        let left = fits(chest[0] - GAP);
        let right = fits(size[0] - (chest[0] + chest[2]) - GAP);
        let (columns, x) = if right >= MIN_COLUMNS {
            (right, chest[0] + chest[2] + GAP)
        } else if left >= MIN_COLUMNS {
            (left, chest[0] - GAP - width(left))
        } else {
            return None;
        };
        let button_rows = BUTTONS.len().div_ceil(columns);
        let below = BAR_GAP + BAR_H + BAR_GAP + button_rows as f32 * SLOT + MARGIN;
        let rows = ((chest[3] - TOP - below) / SLOT).floor().max(1.0) as usize;
        let tabs = TABS.len() as f32 * SLOT;
        if TOP + tabs + MARGIN > chest[3] {
            return None;
        }
        Some(Panel { rect: [x, chest[1], width(columns), chest[3]], columns, rows, button_rows })
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

    fn tab(&self, i: usize) -> Rect {
        let t = self.track();
        [t[0] + t[2] + 2.0, self.grid()[1] + i as f32 * SLOT, SLOT, SLOT]
    }

    fn bar(&self) -> Rect {
        let [gx, gy] = self.grid();
        [gx, gy + self.rows as f32 * SLOT + BAR_GAP, self.columns as f32 * SLOT, BAR_H]
    }

    fn button(&self, i: usize) -> Rect {
        let b = self.bar();
        let (row, col) = (i / self.columns, i % self.columns);
        [b[0] + col as f32 * SLOT, b[1] + BAR_H + BAR_GAP + row as f32 * SLOT, SLOT, SLOT]
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
    /// Entry indices (into the view) in the order shown, after tab, search and sort.
    shown: Vec<usize>,
    sort: Sort,
    reverse: bool,
    tab: Option<Tab>,
    row: usize,
    panel: Option<Panel>,
    widgets: Option<(Widgets, WidgetId)>,
    query: String,
    /// Each id's last total and when it changed (a view counter), for the "recent" order.
    changed: HashMap<String, (u64, u64)>,
    views: u64,
    /// While shift-clicking the list, the order stays as it was so entries don't jump; it
    /// resorts when the mouse leaves the panel.
    frozen: bool,
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

/// The server sent a view: note what changed; names and order are made on the next frame.
pub fn viewed(view: View) {
    ui(|u| {
        u.views += 1;
        for (i, id) in view.ids.iter().enumerate() {
            let now = total(&view, i);
            let seen = u.changed.entry(id.clone()).or_insert((now, 0));
            if seen.0 != now {
                *seen = (now, u.views);
            }
        }
        u.view = Some(view);
        u.names.clear();
    });
}

/// The chest closed: forget its view (search, tab and sort stay for the next chest).
pub fn closed() {
    ui(|u| {
        u.view = None;
        u.panel = None;
        u.widgets = None;
        u.row = 0;
        u.frozen = false;
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

fn in_tab(view: &View, i: usize, tab: Option<Tab>) -> bool {
    match tab {
        None | Some(Tab::All) => true,
        Some(Tab::Of(c)) => view.categories.get(i).and_then(|&n| Category::from_index(n)) == Some(c),
        Some(Tab::Mods) => !view.ids[i].starts_with("minecraft:"),
    }
}

/// Names, tab, search and sort for the current view (after a new view, a keystroke or a choice).
/// While frozen, entries keep the places they had; new kinds go at the end.
fn reorder(host: &dyn ClientHost, u: &mut Ui) {
    let Some(view) = &u.view else { return };
    if u.names.len() != view.ids.len() {
        u.names = view.ids.iter().map(|id| item_name(host, id)).collect();
    }
    let all: Vec<Shown> = view
        .ids
        .iter()
        .enumerate()
        .map(|(i, id)| Shown { name: &u.names[i], id, total: total(view, i), changed: u.changed.get(id).map_or(0, |c| c.1) })
        .collect();
    let has_tag = |id: &str, tag: &str| {
        host.registry_entry("minecraft:item", id).and_then(|e| host.registry_entry_in_tag(&e, tag)).unwrap_or(false)
    };
    let query = u.query.clone();
    let keep = |i: usize| in_tab(view, i, u.tab) && entries::matches(&query, &all[i], |t| has_tag(all[i].id, t));
    let fresh: Vec<usize> = entries::order(&all, u.sort, u.reverse).into_iter().filter(|&i| keep(i)).collect();
    u.shown = if u.frozen {
        let old: Vec<&str> = u.shown.iter().filter_map(|&i| u.view.as_ref().and_then(|v| v.ids.get(i)).map(String::as_str)).collect();
        let place = |i: &usize| old.iter().position(|id| *id == view.ids[*i]).unwrap_or(usize::MAX);
        let mut kept = fresh.clone();
        kept.sort_by_key(place);
        kept
    } else {
        fresh
    };
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

/// A slot-look button (one grammar for the panel): the slot frame, the icon, the game's white
/// hover; `on` marks a selected tab with a lighter well.
fn slot_button(host: &dyn ClientHost, draw: &mut dyn DrawContext, rect: Rect, id: &str, hovered: bool, on: bool) {
    frame(draw, [rect[0], rect[1]]);
    if on {
        draw.fill([rect[0] + 1.0, rect[1] + 1.0, 16.0, 16.0], 0xFFC6_C6C6);
    }
    icon(host, draw, id, [rect[0] + 1.0, rect[1] + 1.0]);
    if hovered {
        draw.fill([rect[0] + 1.0, rect[1] + 1.0, 16.0, 16.0], 0x80FF_FFFF);
    }
}

/// The order button's icon: the order the list is in.
fn sort_icon(sort: Sort) -> &'static str {
    match sort {
        Sort::Count => "minecraft:bundle",
        Sort::Name => "minecraft:name_tag",
        Sort::Mod => "minecraft:book",
        Sort::Id => "minecraft:paper",
        Sort::Recent => "minecraft:clock",
    }
}

fn button_icon(b: Button, sort: Sort) -> &'static str {
    match b {
        Button::Tidy => "minecraft:brush",
        Button::Match => "minecraft:hopper",
        Button::Restock => "minecraft:dropper",
        Button::Empty => "minecraft:chest",
        Button::Order => sort_icon(sort),
    }
}

/// The buttons this player has, in order with no gaps (quick deposit is an earned comfort).
fn buttons(view: &View) -> Vec<Button> {
    BUTTONS.into_iter().filter(|b| *b != Button::Empty || view.comforts.0 & 2 != 0).collect()
}

/// Whether the panel shows: a container screen is open and its chest is joined to others.
fn showing(screen: &ScreenView, u: &Ui) -> bool {
    screen.menu.is_some() && u.view.as_ref().is_some_and(|v| v.containers.0 > 1)
}

/// Slots used and slots in all.
fn capacity(view: &View) -> (u64, u64) {
    let get = |i: usize| view.slots.get(i).map_or(0, |v| v.0.max(0) as u64);
    (get(0), get(1))
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
        // The search field, made once per chest opened, showing the search kept from the last one.
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
        // The category tabs.
        let tab = u.tab.unwrap_or(Tab::All);
        for (i, (t, item, _)) in TABS.iter().enumerate() {
            let r = panel.tab(i);
            slot_button(host, draw, r, item, inside(r, screen.mouse), *t == tab);
        }
        // The capacity bar: green, amber past 75%, red past 90%.
        let (used, all) = capacity(&view);
        let bar = panel.bar();
        draw.fill(bar, 0xFF37_3737);
        if all > 0 {
            let share = used as f32 / all as f32;
            let colour = if share > 0.9 { 0xFFE0_4040 } else if share > 0.75 { 0xFFE0_A030 } else { 0xFF50_C050 };
            draw.fill([bar[0], bar[1], (bar[2] * share).round(), bar[3]], colour);
        }
        // The buttons.
        for (i, b) in buttons(&view).into_iter().enumerate() {
            let r = panel.button(i);
            slot_button(host, draw, r, button_icon(b, u.sort), inside(r, screen.mouse), false);
        }
    });
}

fn sort_key(sort: Sort) -> &'static str {
    match sort {
        Sort::Count => "storage.sort.count",
        Sort::Name => "storage.sort.name",
        Sort::Mod => "storage.sort.mod",
        Sort::Id => "storage.sort.id",
        Sort::Recent => "storage.sort.recent",
    }
}

/// Hover: the list slot's highlight and tooltip, and every control's tooltip, over everything.
pub fn hover(host: &dyn ClientHost, draw: &mut dyn DrawContext, screen: &ScreenView) {
    ui(|u| {
        let Some(panel) = u.panel else { return };
        if !showing(screen, u) {
            return;
        }
        let mouse = screen.mouse;
        // Leaving the panel lets a frozen order settle.
        if u.frozen && !inside(panel.rect, mouse) {
            u.frozen = false;
            reorder(host, u);
        }
        let Some(view) = u.view.clone() else { return };
        let line = |key: &str, fallback: &str| Text::literal(tr(host, key, fallback));
        let lines: Vec<Text> = if let Some(pos) = panel.slot_at(mouse) {
            let [gx, gy] = panel.grid();
            let at = [gx + 1.0 + (pos % panel.columns) as f32 * SLOT, gy + 1.0 + (pos / panel.columns) as f32 * SLOT];
            draw.fill([at[0], at[1], 16.0, 16.0], 0x80FF_FFFF);
            match u.shown.get(u.row * panel.columns + pos) {
                Some(&i) => {
                    let mut l = vec![Text::literal(u.names[i].clone()), Text::literal(total(&view, i).to_string())];
                    if view.comforts.0 & 1 != 0 {
                        l.push(line("storage.find_hint", "F: show the chest"));
                    }
                    l
                }
                None => vec![],
            }
        } else if let Some((_, _, key)) = TABS.iter().enumerate().find(|(i, _)| inside(panel.tab(*i), mouse)).map(|(_, t)| t) {
            vec![line(key, key)]
        } else if inside([panel.bar()[0], panel.bar()[1] - 1.0, panel.bar()[2], panel.bar()[3] + 2.0], mouse) {
            let (used, all) = capacity(&view);
            let text = tr(host, "storage.capacity", "%1$s / %2$s slots, %3$s chests")
                .replace("%1$s", &used.to_string())
                .replace("%2$s", &all.to_string())
                .replace("%3$s", &view.containers.0.to_string());
            vec![Text::literal(text)]
        } else if let Some(b) = buttons(&view).into_iter().enumerate().find(|(i, _)| inside(panel.button(*i), mouse)).map(|(_, b)| b) {
            match b {
                Button::Tidy => vec![line("storage.tidy", "Sort the chests")],
                Button::Match => vec![line("storage.match", "Put in what storage already holds")],
                Button::Restock => vec![line("storage.restock", "Fill your stacks from storage")],
                Button::Empty => vec![line("storage.empty", "Put everything in (not the hotbar)")],
                Button::Order => {
                    let dir = if u.reverse { "storage.sort.down" } else { "storage.sort.up" };
                    vec![line(sort_key(u.sort), sort_key(u.sort)), line(dir, dir)]
                }
            }
        } else {
            vec![]
        };
        if !lines.is_empty() {
            let _ = draw.gui_tooltip(&lines, mouse);
        }
    });
}

/// A press on the panel: the list, the search field, a tab or a button. Presses elsewhere are
/// the screen's own.
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
        let Some(view) = u.view.clone() else { return true };
        let action = |kind: i32| Request::Comfort(Comfort { kind: VarInt(kind), index: VarInt(-1), id: String::new() });
        if let Some(pos) = panel.slot_at(mouse) {
            let index = u.shown.get(u.row * panel.columns + pos).copied();
            let id = index.map(|i| view.ids[i].clone()).unwrap_or_default();
            let index = index.map_or(-1, |i| i as i32);
            u.frozen |= shift;
            u.outbox.push(Request::Click(Click { index: VarInt(index), id, button: VarInt(button), shift }));
        } else if let Some(i) = (0..TABS.len()).find(|i| inside(panel.tab(*i), mouse)) {
            u.tab = Some(TABS[i].0);
            u.row = 0;
            reorder(host, u);
        } else if let Some(b) = buttons(&view).into_iter().enumerate().find(|(i, _)| inside(panel.button(*i), mouse)).map(|(_, b)| b) {
            match b {
                Button::Tidy => u.outbox.push(action(comfort::SORT)),
                Button::Match => u.outbox.push(action(comfort::MATCH)),
                Button::Restock => u.outbox.push(action(comfort::RESTOCK)),
                Button::Empty => u.outbox.push(action(comfort::EMPTY)),
                Button::Order => {
                    if button == 1 {
                        u.reverse = !u.reverse;
                    } else {
                        u.sort = u.sort.next();
                    }
                    u.frozen = false;
                    reorder(host, u);
                }
            }
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

    fn within(inner: Rect, outer: Rect) -> bool {
        inner[0] >= outer[0] && inner[1] >= outer[1] && inner[0] + inner[2] <= outer[0] + outer[2] && inner[1] + inner[3] <= outer[1] + outer[3]
    }

    #[test]
    fn the_panel_and_all_its_controls_fit_beside_the_chest_at_every_width() {
        for w in [400.0, 427.0, 480.0, 533.0, 640.0, 960.0] {
            let Some(p) = Panel::beside(chest(w), [w, 300.0]) else { panic!("no room at {w}") };
            let c = chest(w);
            assert!(p.rect[0] >= 0.0 && p.rect[0] + p.rect[2] <= w, "{w}: {:?}", p.rect);
            assert!(p.rect[0] + p.rect[2] <= c[0] || p.rect[0] >= c[0] + c[2], "{w}: overlaps the chest");
            assert!((MIN_COLUMNS..=MAX_COLUMNS).contains(&p.columns));
            assert!(p.rows >= 3, "{w}: only {} rows", p.rows);
            for i in 0..TABS.len() {
                assert!(within(p.tab(i), p.rect), "{w}: tab {i} outside");
            }
            for i in 0..BUTTONS.len() {
                assert!(within(p.button(i), p.rect), "{w}: button {i} outside {:?} {:?}", p.button(i), p.rect);
            }
            assert!(within(p.bar(), p.rect));
            assert!(within(p.track(), p.rect));
        }
    }

    #[test]
    fn narrow_panels_wrap_their_buttons() {
        // Just room for 3 columns on the right: 50 pixels of frame and scrollbar, 3 slots.
        let p = Panel::beside([200.0, 20.0, 176.0, 166.0], [200.0 + 176.0 + 2.0 + 50.0 + 54.0, 270.0]).unwrap();
        assert_eq!(p.columns, 3);
        assert_eq!(p.button_rows, 2);
        let wide = Panel::beside([0.0, 20.0, 176.0, 222.0], [600.0, 270.0]).unwrap();
        assert_eq!(wide.button_rows, 1);
        assert!(wide.rows > p.rows);
    }
}

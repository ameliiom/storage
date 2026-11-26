//! The client: when a chest (or a station) joined to others is open, a panel beside the game's own
//! screen shows everything in all of them as one list, with search, category tabs, sort orders,
//! a capacity bar and the organizing buttons. The chest screen, its slots, the inventory and the
//! cursor stay the game's own; the panel is drawn in the game's look (its panel, slot frames, the
//! creative search field and scrollbar, item counts at half size as storage mods show big counts).
//! Every click on it is a request the server answers with a new view.
use crate::entries::{self, Pin, Shown, Sort};
use crate::nbtjson;
use crate::organize::Category;
use crate::payloads::{self, Click, Comfort, Near, View, action, comfort};
use mod_api::client::hud_layout::{Blend, PoseOp};
use mod_api::client::widgets::{Outcome, TextField, WidgetId, Widgets};
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{ClientHost, ConfigPath, DrawContext, GuiDraw, Identifier, ItemResource, ItemStack, ScreenView, Text, TooltipElement, VarInt};
use std::collections::{HashMap, HashSet};
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
/// The gap between the chest's screen and the panel, and between the panel and the screen's edge.
const GAP: f32 = 2.0;
const EDGE: f32 = 4.0;
/// The clear button at the search field's end.
const CLEAR_W: f32 = 11.0;
/// Two presses on one entry closer than this are a double-click.
const DOUBLE_MS: u64 = 300;
/// The pinned item ids, one a line (with a minimum after a space), in the game's `config/` folder.
const PINS: &str = "storage-pins.txt";
/// `View::comforts` bits the panel reads (`skills::COMFORTS` order).
const FIND_BIT: i32 = 1;
const EMPTY_BIT: i32 = 2;
const LOW_BIT: i32 = 16;

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
    /// else to its left, as wide as fits (3 to 9 columns), the whole screen's height.
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
        let (top, height) = if size[1] - 2.0 * EDGE > chest[3] { (EDGE, size[1] - 2.0 * EDGE) } else { (chest[1], chest[3]) };
        let button_rows = BUTTONS.len().div_ceil(columns);
        let below = BAR_GAP + BAR_H + BAR_GAP + button_rows as f32 * SLOT + MARGIN;
        let rows = ((height - TOP - below) / SLOT).floor().max(1.0) as usize;
        let tabs = TABS.len() as f32 * SLOT;
        if TOP + tabs + MARGIN > height {
            return None;
        }
        Some(Panel { rect: [x, top, width(columns), height], columns, rows, button_rows })
    }

    fn grid(&self) -> [f32; 2] {
        [self.rect[0] + MARGIN, self.rect[1] + TOP]
    }

    fn search(&self) -> Rect {
        [self.rect[0] + MARGIN, self.rect[1] + 3.0, self.columns as f32 * SLOT - CLEAR_W - 1.0, 12.0]
    }

    fn clear(&self) -> Rect {
        let s = self.search();
        [s[0] + s[2] + 1.0, s[1], CLEAR_W, s[3]]
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
    /// Entry indices (into `ids`) in the order shown, after tab, search and sort.
    shown: Vec<usize>,
    /// The ids in the order last shown: a frozen list keeps them in place across new views,
    /// whose indices may differ.
    order: Vec<String>,
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
    /// Each entry's stack, rebuilt from the view: drawn with its glint and durability bar.
    stacks: Vec<Option<ItemStack>>,
    /// Each entry's tooltip in lower case, filled when a search has a `$` word.
    tips: Vec<String>,
    /// Pinned items (they come first), read from `PINS` once.
    pins: Option<Vec<Pin>>,
    /// The ids shown: the view's, then pinned items with a minimum that storage has none of
    /// (their totals are 0, so they show red). Indices past the view's are these ghosts.
    ids: Vec<String>,
    /// The own-inventory panel was asked for (it is put away when the inventory closes).
    near: bool,
    /// Item ids whose variants are shown one by one; others show as one group.
    open: HashSet<String>,
    /// The group heads shown: entry index → (total of the group, its kinds).
    groups: HashMap<usize, (u64, usize)>,
    /// The last left press on the list: entry index and when, for double-clicks.
    last: Option<(usize, u64)>,
}

enum Request {
    Click(Click),
    Comfort(Comfort),
    Near(Near),
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
        u.open.clear();
        u.last = None;
        u.ids.clear();
        if std::mem::take(&mut u.near) {
            u.outbox.push(Request::Near(Near { open: false }));
        }
    });
}

/// The player's own inventory (not a chest, a station or creative), where the panel shows the
/// chests within reach for those with the comfort.
fn own_inventory(screen: &ScreenView) -> bool {
    screen.id == "inventory" && screen.menu.is_none()
}

fn load_pins(host: &dyn ClientHost) -> Vec<Pin> {
    let Ok(path) = ConfigPath::new(PINS) else { return Vec::new() };
    let Ok(bytes) = host.config_files().read(&path) else { return Vec::new() };
    entries::read_pins(&String::from_utf8_lossy(&bytes))
}

fn save_pins(host: &dyn ClientHost, pins: &[Pin]) {
    let Ok(path) = ConfigPath::new(PINS) else { return };
    if let Err(e) = host.config_files().write(&path, entries::write_pins(pins).as_bytes()) {
        eprintln!("[storage] keeping the pins: {e:?}");
    }
}

fn pin_of<'a>(u: &'a Ui, id: &str) -> Option<&'a Pin> {
    u.pins.as_ref()?.iter().find(|p| p.id == id)
}

/// A stack described by the server (`json`), else the plain item `id`.
fn stack_of(host: &dyn ClientHost, json: Option<&String>, id: &str) -> Option<ItemStack> {
    let described = json.filter(|s| !s.is_empty()).and_then(|s| nbtjson::read(s));
    if let Some(stack) = described.and_then(|c| ItemStack::from_nbt(host, &c)) {
        return Some(stack);
    }
    Identifier::parse(id).ok().and_then(|k| host.item_key(&k)).map(ItemStack::of)
}

/// The stack's tooltip lines as the game writes them (name, enchantments, effects, lore).
fn tooltip(host: &dyn ClientHost, stack: &ItemStack) -> Vec<Text> {
    ItemResource::of(stack).tooltip(host, false, false)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|e| match e {
            TooltipElement::Text(t, _) => Some(t),
            _ => None,
        })
        .collect()
}

pub fn tick(host: &mut dyn ClientHost) {
    for request in ui(|u| std::mem::take(&mut u.outbox)) {
        let sent = match request {
            Request::Click(r) => payloads::CLICK.get().map(|t| t.send(host, &r)),
            Request::Comfort(r) => payloads::COMFORT.get().map(|t| t.send(host, &r)),
            Request::Near(r) => payloads::NEAR.get().map(|t| t.send(host, &r)),
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

use crate::kit::lang::tr;

fn total(view: &View, i: usize) -> u64 {
    view.totals.get(i).map_or(0, |t| t.0.max(0) as u64)
}

fn in_tab(view: &View, ids: &[String], i: usize, tab: Option<Tab>) -> bool {
    match tab {
        None | Some(Tab::All) => true,
        Some(Tab::Of(c)) => view.categories.get(i).and_then(|&n| Category::from_index(n)) == Some(c),
        Some(Tab::Mods) => !ids[i].starts_with("minecraft:"),
    }
}

/// Names, tab, search, pins, groups and sort for the current view (after a new view, a keystroke
/// or a choice). While frozen, entries keep the places they had; new kinds go at the end.
fn reorder(host: &dyn ClientHost, u: &mut Ui) {
    let Some(view) = &u.view else { return };
    let pins = u.pins.get_or_insert_with(|| load_pins(host)).clone();
    // Pinned items with a minimum that storage has none of still show, empty and red.
    let low_marks = view.comforts.0 & LOW_BIT != 0;
    let ghosts = pins.iter().filter(|p| low_marks && p.min > 0 && !view.ids.contains(&p.id)).map(|p| p.id.clone());
    let ids: Vec<String> = view.ids.iter().cloned().chain(ghosts).collect();
    if u.ids != ids || u.names.len() != ids.len() {
        u.names = ids.iter().map(|id| item_name(host, id)).collect();
        u.stacks = ids.iter().enumerate().map(|(i, id)| stack_of(host, view.stacks.get(i), id)).collect();
        u.tips.clear();
        u.ids = ids;
    }
    let ids = &u.ids;
    let query = u.query.clone();
    if u.tips.len() != ids.len() && query.split_whitespace().any(|w| w.starts_with('$')) {
        // The tooltip in the player's language, then the stack's data (enchantment and effect
        // ids), so `$sharp` finds Sharpness in any language.
        u.tips = u
            .stacks
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let lines = s.as_ref().map(|s| tooltip(host, s)).unwrap_or_default();
                let mut text = lines.iter().filter_map(|t| host.text_string(t).ok()).collect::<Vec<_>>().join("\n");
                text.push('\n');
                text.push_str(view.stacks.get(i).map_or("", String::as_str));
                text.to_lowercase()
            })
            .collect();
    }
    let all: Vec<Shown> = ids
        .iter()
        .enumerate()
        .map(|(i, id)| Shown {
            name: &u.names[i],
            id,
            total: total(view, i),
            changed: u.changed.get(id).map_or(0, |c| c.1),
            tip: u.tips.get(i).map_or("", String::as_str),
        })
        .collect();
    let has_tag = |id: &str, tag: &str| {
        host.registry_entry("minecraft:item", id).and_then(|e| host.registry_entry_in_tag(&e, tag)).unwrap_or(false)
    };
    let keep = |i: usize| in_tab(view, ids, i, u.tab) && entries::matches(&query, &all[i], |t| has_tag(all[i].id, t));
    let mut fresh: Vec<usize> = entries::order(&all, u.sort, u.reverse).into_iter().filter(|&i| keep(i)).collect();
    // Pinned items first, in the order otherwise chosen.
    fresh.sort_by_key(|&i| !pins.iter().any(|p| p.id == ids[i]));
    let totals: Vec<u64> = (0..ids.len()).map(|i| total(view, i)).collect();
    let (fresh, groups) = entries::group(ids, &totals, fresh, &u.open);
    u.groups = groups;
    u.shown = if u.frozen {
        let old = &u.order;
        let place = |i: &usize| old.iter().position(|id| *id == ids[*i]).unwrap_or(usize::MAX);
        let mut kept = fresh.clone();
        kept.sort_by_key(place);
        kept
    } else {
        fresh
    };
    u.order = u.shown.iter().map(|&i| u.ids[i].clone()).collect();
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

pub(crate) fn sprite(draw: &mut dyn DrawContext, name: &str, [x, y, w, h]: Rect) {
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
    BUTTONS.into_iter().filter(|b| *b != Button::Empty || view.comforts.0 & EMPTY_BIT != 0).collect()
}

/// Whether the panel shows: a container screen (or the own inventory beside chests) is open and
/// its chest is joined to others.
fn showing(screen: &ScreenView, u: &Ui) -> bool {
    (screen.menu.is_some() || own_inventory(screen)) && u.view.as_ref().is_some_and(|v| v.containers.0 > 1)
}

/// Slots used and slots in all.
fn capacity(view: &View) -> (u64, u64) {
    let get = |i: usize| view.slots.get(i).map_or(0, |v| v.0.max(0) as u64);
    (get(0), get(1))
}

/// Draws the panel under the screen's own items, tooltips and cursor stack.
/// Where the bag's sort button sits, if this screen shows the player's inventory: top right of the
/// own inventory (beside the crafting grid's title), or at the end of a chest's "Inventory" row.
fn bag_button(screen: &ScreenView) -> Option<Rect> {
    let [x, y, w, h] = screen.rect;
    if own_inventory(screen) {
        return Some([x + w - 16.0, y + 5.0, 11.0, 11.0]);
    }
    let chest = screen.menu.is_some_and(|m| m.starts_with("minecraft:generic_") || m == "minecraft:shulker_box");
    chest.then_some([x + w - 18.0, y + h - 97.0, 11.0, 11.0])
}

/// The sort button: a small slot with the brush, the panel's own grammar.
fn draw_bag_button(host: &dyn ClientHost, draw: &mut dyn DrawContext, screen: &ScreenView) {
    let Some([x, y, w, h]) = bag_button(screen) else { return };
    draw.fill([x, y, w, 1.0], 0xFF37_3737);
    draw.fill([x, y, 1.0, h], 0xFF37_3737);
    draw.fill([x + 1.0, y + h - 1.0, w - 1.0, 1.0], 0xFFFF_FFFF);
    draw.fill([x + w - 1.0, y + 1.0, 1.0, h - 1.0], 0xFFFF_FFFF);
    draw.fill([x + 1.0, y + 1.0, w - 2.0, h - 2.0], 0xFF8B_8B8B);
    if let Some(key) = Identifier::parse("minecraft:brush").ok().and_then(|k| host.item_key(&k)) {
        draw.item(key, [x + 1.0, y + 1.0], w - 2.0);
    }
    if inside([x, y, w, h], screen.mouse) {
        draw.fill([x + 1.0, y + 1.0, w - 2.0, h - 2.0], 0x80FF_FFFF);
    }
}

pub fn draw(host: &dyn ClientHost, draw: &mut dyn DrawContext, screen: &ScreenView) {
    draw_bag_button(host, draw, screen);
    ui(|u| {
        // The own inventory asks once whether chests are within reach (the server answers with
        // a view only for those with the comfort).
        if own_inventory(screen) && !u.near {
            u.near = true;
            // Only a view answering this request shows (one in flight from a chest just closed
            // would be stale).
            u.view = None;
            u.names.clear();
            u.outbox.push(Request::Near(Near { open: true }));
        }
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
        // The clear button: an x at the field's end while there is a search.
        if !u.query.is_empty() {
            let r = panel.clear();
            let colour = if inside(r, screen.mouse) { 0xFFFF_FFFF } else { 0xFF40_4040 };
            draw.text("x", [r[0] + (r[2] - draw.text_width("x")) / 2.0, r[1] + 2.0], colour, false);
        }
        let [gx, gy] = panel.grid();
        let low_marks = view.comforts.0 & LOW_BIT != 0;
        for r in 0..panel.rows {
            for c in 0..panel.columns {
                let at = [gx + c as f32 * SLOT, gy + r as f32 * SLOT];
                frame(draw, at);
                let Some(&i) = u.shown.get((u.row + r) * panel.columns + c) else { continue };
                let inner = [at[0] + 1.0, at[1] + 1.0];
                let group = u.groups.get(&i).copied();
                let n = group.map_or(total(&view, i), |g| g.0);
                let pin = pin_of(u, &u.ids[i]).cloned();
                // Below its minimum: the well turns red, as a hurt heart flashes.
                if low_marks && pin.as_ref().is_some_and(|p| entries::low(p.min, n)) {
                    draw.fill([inner[0], inner[1], 16.0, 16.0], 0xFFC0_3030);
                }
                match u.stacks.get(i).and_then(Option::as_ref) {
                    Some(stack) => {
                        draw.item_stack(stack, inner, 16.0);
                    }
                    None => icon(host, draw, &u.ids[i], inner),
                }
                if n > 1 {
                    count(draw, n, at);
                }
                // A group's mark: a small plus at the top right, as a bundle hints at more inside.
                if group.is_some() {
                    draw.fill([inner[0] + 11.0, inner[1] + 2.0, 5.0, 1.0], 0xFFFF_FFFF);
                    draw.fill([inner[0] + 13.0, inner[1], 1.0, 5.0], 0xFFFF_FFFF);
                }
                // A pin's mark: a gold corner at the top left.
                if pin.is_some() {
                    draw.fill([inner[0], inner[1], 3.0, 1.0], 0xFFFF_D23C);
                    draw.fill([inner[0], inner[1], 1.0, 3.0], 0xFFFF_D23C);
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
    if bag_button(screen).is_some_and(|r| inside(r, screen.mouse)) {
        let _ = draw.gui_tooltip(&[Text::literal(tr(host, "storage.sort_bag", "Sort your bag"))], screen.mouse);
        return;
    }
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
                    // The game's own tooltip for the stack (enchantments, durability, effects).
                    let mut l = u.stacks.get(i).and_then(Option::as_ref).map(|s| tooltip(host, s)).unwrap_or_default();
                    if l.is_empty() {
                        l.push(Text::literal(u.names[i].clone()));
                    }
                    let fill = |key: &str, fallback: &str, args: &[String]| {
                        args.iter().enumerate().fold(tr(host, key, fallback), |t, (n, a)| t.replace(&format!("%{}$s", n + 1), a))
                    };
                    match u.groups.get(&i) {
                        Some(&(sum, kinds)) => l.push(Text::literal(fill(
                            "storage.group",
                            "%1$s kinds, %2$s in all: click to show them",
                            &[kinds.to_string(), sum.to_string()],
                        ))),
                        None => l.push(Text::literal(fill("storage.total", "In storage: %1$s", &[total(&view, i).to_string()]))),
                    }
                    if view.comforts.0 & FIND_BIT != 0 && i < view.ids.len() {
                        l.push(line("storage.find_hint", "F: show the chest"));
                    }
                    let pin = pin_of(u, &u.ids[i]).cloned();
                    if view.comforts.0 & LOW_BIT != 0
                        && let Some(p) = &pin
                    {
                        l.push(Text::literal(fill("storage.minimum", "Keep at least %1$s (Ctrl + wheel)", &[p.min.to_string()])));
                    }
                    l.push(if pin.is_some() { line("storage.unpin", "Middle-click: unpin") } else { line("storage.pin", "Middle-click: pin") });
                    l
                }
                None => vec![],
            }
        } else if !u.query.is_empty() && inside(panel.clear(), mouse) {
            vec![line("storage.clear", "Clear the search")]
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
        if button == 0 && bag_button(screen).is_some_and(|r| inside(r, mouse)) {
            u.outbox.push(Request::Comfort(Comfort { kind: VarInt(comfort::BAG), index: VarInt(-1), id: String::new() }));
            return true;
        }
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
        if !u.query.is_empty() && inside(panel.clear(), mouse) {
            u.query.clear();
            if let Some((widgets, id)) = &mut u.widgets
                && let Some(f) = widgets.text_field_mut(*id)
            {
                f.set_value("");
            }
            u.row = 0;
            reorder(host, u);
        } else if let Some(pos) = panel.slot_at(mouse) {
            let index = u.shown.get(u.row * panel.columns + pos).copied();
            // Middle-click pins (or unpins) the item.
            if button == 2 {
                if let Some(i) = index {
                    let id = u.ids[i].clone();
                    let pins = u.pins.get_or_insert_with(|| load_pins(host));
                    match pins.iter().position(|p| p.id == id) {
                        Some(at) => drop(pins.remove(at)),
                        None => pins.push(Pin { id, min: 0 }),
                    }
                    save_pins(host, pins);
                    reorder(host, u);
                }
                return true;
            }
            // A group opens to show its variants one by one.
            if let Some(i) = index
                && u.groups.contains_key(&i)
            {
                u.open.insert(u.ids[i].clone());
                reorder(host, u);
                return true;
            }
            // A second left press on the same entry, quickly: as much of it as the inventory holds.
            let now = host.millis();
            let double = button == 0
                && !shift
                && index.is_some_and(|i| u.last.is_some_and(|(j, t)| j == i && now.saturating_sub(t) < DOUBLE_MS));
            u.last = if button == 0 && !double { index.map(|i| (i, now)) } else { None };
            u.frozen |= shift;
            let kind = if double { action::ALL } else { action::CLICK };
            u.outbox.push(click(&view, index, button, shift, kind));
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

/// A request about entry `i` (none, or a ghost past the view's entries: the empty space after
/// the list, where a click puts the cursor in).
fn click(view: &View, index: Option<usize>, button: i32, shift: bool, kind: i32) -> Request {
    let index = index.filter(|&i| i < view.ids.len());
    let id = index.map(|i| view.ids[i].clone()).unwrap_or_default();
    let index = VarInt(index.map_or(-1, |i| i as i32));
    Request::Click(Click { index, id, button: VarInt(button), shift, action: VarInt(kind) })
}

/// The entry under the mouse, if the panel shows one there (ghosts included).
fn hovered(u: &Ui, mouse: [f32; 2]) -> Option<usize> {
    let panel = u.panel?;
    u.shown.get(u.row * panel.columns + panel.slot_at(mouse)?).copied()
}

/// The entry under the mouse that storage holds (not a ghost).
fn held_entry(u: &Ui, mouse: [f32; 2]) -> Option<usize> {
    hovered(u, mouse).filter(|&i| u.view.as_ref().is_some_and(|v| i < v.ids.len()))
}

/// The wheel scrolls the list; with shift over an entry it moves one at a time: down takes one
/// into the inventory, up puts one back. With Ctrl over a pinned entry (and the comfort), it
/// raises or lowers the entry's minimum by a stack.
pub fn scroll(host: &dyn ClientHost, screen: &ScreenView, by: f64, modifiers: u32) -> bool {
    ui(|u| {
        let Some(panel) = u.panel else { return false };
        if !showing(screen, u) || !inside(panel.rect, screen.mouse) {
            return false;
        }
        let low_marks = u.view.as_ref().is_some_and(|v| v.comforts.0 & LOW_BIT != 0);
        if modifiers & 2 != 0 && low_marks {
            if let Some(i) = hovered(u, screen.mouse) {
                let id = u.ids[i].clone();
                let stack = Identifier::parse(&id).ok().and_then(|k| host.item_key(&k)).and_then(|k| host.max_stack_size(k)).unwrap_or(64);
                let pins = u.pins.get_or_insert_with(|| load_pins(host));
                if let Some(p) = pins.iter_mut().find(|p| p.id == id) {
                    p.min = if by > 0.0 { p.min + u64::from(stack) } else { p.min.saturating_sub(u64::from(stack)) };
                    save_pins(host, pins);
                    reorder(host, u);
                }
            }
            return true;
        }
        if modifiers & 1 != 0 {
            if let (Some(i), Some(view)) = (held_entry(u, screen.mouse), &u.view)
                && !u.groups.contains_key(&i)
            {
                let kind = if by < 0.0 { action::ONE_OUT } else { action::ONE_IN };
                let request = click(view, Some(i), 0, false, kind);
                u.frozen = true;
                u.outbox.push(request);
            }
            return true;
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
        let ctrl = modifiers & 2 != 0;
        // Ctrl+F: to the search field.
        if key == "key.keyboard.f" && ctrl {
            if let Some((widgets, id)) = &mut u.widgets {
                widgets.set_focus(host, Some(*id));
            }
            return true;
        }
        let Some(view) = &u.view else { return false };
        // Q over an entry drops one in front of you, Ctrl+Q a stack, as from a slot.
        if key == "key.keyboard.q"
            && let Some(i) = held_entry(u, screen.mouse)
            && !u.groups.contains_key(&i)
        {
            let request = click(view, Some(i), 0, false, if ctrl { action::DROP_STACK } else { action::DROP });
            u.outbox.push(request);
            return true;
        }
        if key == "key.keyboard.f"
            && view.comforts.0 & FIND_BIT != 0
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
            assert!(within(p.search(), p.rect) && within(p.clear(), p.rect), "{w}: search row outside");
            assert!(p.rect[1] >= 0.0 && p.rect[1] + p.rect[3] <= 300.0, "{w}: taller than the screen");
            assert!(p.rect[3] > c[3], "{w}: not using the screen height");
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

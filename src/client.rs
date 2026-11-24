//! The client: the storage screen, drawn in the creative inventory's own look (its item-search
//! tab: 9 × 5 slots, the search field, the scrollbar), showing the view the server sent. A click
//! asks the server to take a stack; nothing is moved here.
use crate::entries::count_text;
use crate::payloads::{self, Take, View};
use mod_api::client::hud_layout::Blend;
#[allow(unused_imports)]
use mod_api::prelude::*;
use mod_api::{ClientHost, DrawContext, GuiDraw, Identifier, Text, VarInt};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// The creative inventory's search tab (a path under `textures/`): 195 × 136 on a 256 × 256 sheet.
const PANEL: &str = "gui/container/creative_inventory/tab_item_search";
const PANEL_SIZE: [f32; 2] = [195.0, 136.0];
const COLUMNS: usize = 9;
const ROWS: usize = 5;
/// The first slot's top-left inside the panel, and a slot's pitch.
const GRID: [f32; 2] = [9.0, 18.0];
const SLOT: f32 = 18.0;
const SCROLLER: &str = "container/creative_inventory/scroller";
const SCROLLER_OFF: &str = "container/creative_inventory/scroller_disabled";
/// The scrollbar's track inside the panel: x, top, height; the knob is 12 × 15.
const TRACK: [f32; 3] = [175.0, 18.0, 112.0];

#[derive(Default)]
struct Ui {
    view: Option<View>,
    /// The screen size at the last frame, for input hooks that carry none.
    size: [f32; 2],
    /// The first row shown.
    row: usize,
    /// A take waiting for the next tick (sending needs the mutable host).
    take: Option<Take>,
}

static UI: Mutex<Option<Ui>> = Mutex::new(None);
/// Opening a screen needs the mutable host, so a view only flags it for the next tick.
static OPEN: AtomicBool = AtomicBool::new(false);

fn ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    let mut guard = UI.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Ui::default))
}

fn mine(host: &dyn ClientHost) -> bool {
    host.mod_screen() == Some(crate::MOD_ID)
}

fn rows(view: &View) -> usize {
    view.ids.len().div_ceil(COLUMNS)
}

/// The server sent a view: show it, opening the screen the first time.
pub fn viewed(view: View) {
    ui(|u| {
        u.row = u.row.min(rows(&view).saturating_sub(ROWS));
        u.view = Some(view);
    });
    OPEN.store(true, Ordering::Relaxed);
}

pub fn tick(host: &mut dyn ClientHost) {
    if OPEN.swap(false, Ordering::Relaxed) && !mine(host) {
        let _ = host.mod_screen_open(crate::MOD_ID);
    }
    if let Some(take) = ui(|u| u.take.take())
        && let Some(token) = payloads::TAKE.get()
        && let Err(e) = token.send(host, &take)
    {
        eprintln!("[storage] sending a take: {e:?}");
    }
}

/// The panel's top-left, centred.
fn panel(size: [f32; 2]) -> [f32; 2] {
    [((size[0] - PANEL_SIZE[0]) / 2.0).floor(), ((size[1] - PANEL_SIZE[1]) / 2.0).floor()]
}

/// The entry index under `mouse`, if any.
fn slot_at(size: [f32; 2], row: usize, count: usize, mouse: [f32; 2]) -> Option<usize> {
    let [px, py] = panel(size);
    let (x, y) = (mouse[0] - px - GRID[0], mouse[1] - py - GRID[1]);
    if x < 0.0 || y < 0.0 {
        return None;
    }
    let (col, r) = ((x / SLOT) as usize, (y / SLOT) as usize);
    if col >= COLUMNS || r >= ROWS || x % SLOT >= 16.0 || y % SLOT >= 16.0 {
        return None;
    }
    let index = (row + r) * COLUMNS + col;
    (index < count).then_some(index)
}

fn item_name(host: &dyn ClientHost, id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
    host.translate(&format!("item.{ns}.{path}"))
        .or_else(|| host.translate(&format!("block.{ns}.{path}")))
        .unwrap_or_else(|| id.to_owned())
}

fn total(view: &View, i: usize) -> u64 {
    view.totals.get(i).map_or(0, |t| t.0.max(0) as u64)
}

pub fn draw(host: &dyn ClientHost, draw: &mut dyn DrawContext, mouse: [f32; 2]) {
    if !mine(host) {
        return;
    }
    let size = draw.size();
    let Some((view, row)) = ui(|u| {
        u.size = size;
        u.view.clone().map(|v| (v, u.row))
    }) else {
        return;
    };
    host.apply_screen_blur();
    draw.draw_menu_background([0.0, 0.0, size[0], size[1]], true);
    let [px, py] = panel(size);
    let _ = draw.gui_paint(&[GuiDraw::Blit {
        texture: PANEL.to_owned().into(),
        blend: Blend::Textured,
        x: px as i32,
        y: py as i32,
        w: PANEL_SIZE[0] as i32,
        h: PANEL_SIZE[1] as i32,
        uv: [0.0, 0.0],
        src: [PANEL_SIZE[0] as i32, PANEL_SIZE[1] as i32],
        tex: [256, 256],
        color: 0xFFFF_FFFF,
    }]);
    let title = host.translate("storage.title").unwrap_or_else(|| "Storage".into());
    draw.text(&title, [px + 8.0, py + 6.0], 0xFF40_4040, false);
    for r in 0..ROWS {
        for col in 0..COLUMNS {
            let i = (row + r) * COLUMNS + col;
            let Some(id) = view.ids.get(i) else { break };
            let at = [px + GRID[0] + col as f32 * SLOT, py + GRID[1] + r as f32 * SLOT];
            if let Some(key) = Identifier::parse(id).ok().and_then(|k| host.item_key(&k)) {
                draw.item(key, at, 16.0);
            }
            if total(&view, i) > 1 {
                let count = count_text(total(&view, i));
                let w = draw.text_width(&count);
                draw.text(&count, [at[0] + 17.0 - w, at[1] + 9.0], 0xFFFF_FFFF, true);
            }
        }
    }
    // The scrollbar: the knob at its share of the rows, greyed when everything fits.
    let all = rows(&view);
    let (sprite, share) = if all > ROWS { (SCROLLER, row as f32 / (all - ROWS) as f32) } else { (SCROLLER_OFF, 0.0) };
    let _ = draw.gui_paint(&[GuiDraw::Sprite {
        sprite: sprite.to_owned().into(),
        x: (px + TRACK[0]) as i32,
        y: (py + TRACK[1] + share * (TRACK[2] - 15.0)) as i32,
        w: 12,
        h: 15,
        region: None,
        color: 0xFFFF_FFFF,
        blend: Blend::Textured,
    }]);
    if let Some(i) = slot_at(size, row, view.ids.len(), mouse) {
        let at = [px + GRID[0] + ((i % COLUMNS) as f32) * SLOT, py + GRID[1] + ((i / COLUMNS - row) as f32) * SLOT];
        draw.fill([at[0], at[1], 16.0, 16.0], 0x80FF_FFFF);
        let lines = [Text::literal(item_name(host, &view.ids[i])), Text::literal(total(&view, i).to_string())];
        let _ = draw.gui_tooltip(&lines, mouse);
    }
}

/// A click on an entry takes a stack (64, or half with the right button).
pub fn press(host: &dyn ClientHost, button: i32, mouse: [f32; 2]) -> bool {
    if !mine(host) {
        return false;
    }
    ui(|u| {
        let Some(view) = &u.view else { return };
        if let Some(i) = slot_at(u.size, u.row, view.ids.len(), mouse) {
            let amount = if button == 1 { 32 } else { 64 };
            u.take = Some(Take { index: VarInt(i as i32), id: view.ids[i].clone(), amount: VarInt(amount) });
        }
    });
    true
}

pub fn scroll(host: &dyn ClientHost, by: f64) -> bool {
    if !mine(host) {
        return false;
    }
    ui(|u| {
        let max = u.view.as_ref().map_or(0, |v| rows(v).saturating_sub(ROWS));
        u.row = if by > 0.0 { u.row.saturating_sub(1) } else { (u.row + 1).min(max) };
    });
    true
}

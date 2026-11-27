//! Storage: open any chest joined to others and everything in all of them shows as one list
//! beside it. See `docs/design.md`.
//!
//! - **The chain** (`chain.rs`): which containers a chest is joined to.
//! - **The merged view** (`entries.rs`): one entry per kind, search, sort, splitting a take.
//! - **The server** (`server.rs`): a session per open chest, takes and puts in one transaction.
//! - **The panel** (`client.rs`): beside the chest's own screen, in the game's look.
//! - **Stacks on the wire** (`nbtjson.rs`): an item with components, as typed JSON for the view.
//! - **Comforts** (`skills.rs`): Find, quick deposit and a bigger network, earned in Skills' tree
//!   Organization through tags and Storage's own statistic.
//! - **Shulker peek** (`peek.rs`): a shulker box's slots in its tooltip.
//! - **Furnaces from chests** (`furnace.rs`): furnaces touching a network cook from it.
// Kit (ameliiom/kit), the shared code: the installed Kit mod, or its source in place for
// local builds (`kit_local`, horizon-mods/.cargo/config.toml).
#[cfg(kit_local)]
#[allow(dead_code)]
#[path = "../../kit/src/lib.rs"]
mod kit;
#[cfg(not(kit_local))]
use ::kit;
pub mod chain;
pub mod craft;
pub mod furnace;
pub mod organize;
pub mod peek;
pub(crate) mod shulker;
mod client;
pub mod entries;
pub mod nbtjson;
pub mod payloads;
mod server;
mod skills;

use mod_api::{
    ClientHost, ClientTickPreEvent, EntityJoinLevelEvent, EntityLeaveLevelEvent, Flow, FurnaceBurnEvent, FurnaceExtractEvent, FurnaceSmeltEvent, Hooks, IPayloadContext, PayloadId, PlayerContainerCloseEvent,
    PlayerContainerOpenEvent, PlayerLoggedOutEvent, PlayerRecipeBookClickEvent, RecipeSourcesEvent, Registrar, RegistrationContext, ScreenCharTypedPreEvent,
    ScreenClosingEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenRenderBackgroundEvent, ScreenRenderPostEvent, ServerHost, ServerTickEvent, ServerTickPhase,
};
#[allow(unused_imports)]
use mod_api::prelude::*;

pub(crate) const MOD_ID: &str = "storage";

/// Settings (Options > Mods > Storage).
pub(crate) mod setting {
    use crate::kit::setting::Toggle;
    /// A player's own switch for Shulker peek (client).
    pub const SHULKER_PEEK: Toggle = Toggle::on("shulker_peek");
    /// The server's switch for Furnaces from chests.
    pub const FURNACE_FEED: Toggle = Toggle::on("furnace_feed");
    /// The server's switch for opening a shulker box from the bag.
    pub const SHULKER_OPEN: Toggle = Toggle::on("shulker_open");
    /// Every setting: every comfort without Skills first.
    pub const ALL: [Toggle; 4] = [crate::kit::comforts::ALL, SHULKER_PEEK, FURNACE_FEED, SHULKER_OPEN];
}

fn on_view(view: payloads::View, _: &mut dyn IPayloadContext) -> Result<(), String> {
    client::viewed(view);
    Ok(())
}

fn on_perks(perks: payloads::Perks, _: &mut dyn IPayloadContext) -> Result<(), String> {
    peek::earned(perks.bits.0 & payloads::perk::PEEK != 0, perks.bits.0 & payloads::perk::OPEN_BOX != 0);
    Ok(())
}

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        for key in setting::ALL {
            let _ = r.register_setting(key.spec(MOD_ID));
        }
        r.attach_item_part_tag("minecraft:shulker_boxes", "storage:peek", std::sync::Arc::new(peek::Peek));
        skills::register(r);
        let mut handlers = r.payload_handlers();
        let mut view = handlers.registrar("1").optional();
        let _ = payloads::VIEW.set(view.play_to_client_typed(PayloadId("storage:view".into()), on_view));
        let _ = payloads::CLICK.set(view.play_to_server_typed(PayloadId("storage:click".into()), server::click));
        let _ = payloads::COMFORT.set(view.play_to_server_typed(PayloadId("storage:comfort".into()), skills::comfort));
        let _ = payloads::PERKS.set(view.play_to_client_typed(PayloadId("storage:perks".into()), on_perks));
        let _ = payloads::NEAR.set(view.play_to_server_typed(PayloadId("storage:near".into()), skills::near));
    }

    /// A mod crafting station's recipe book also takes ingredients from the chests touching it
    /// (Horizon asks this for mod crafting menus only, not the vanilla crafting table).
    fn recipe_sources(&self, host: &mut dyn ServerHost, event: &mut RecipeSourcesEvent<'_>) {
        event.sources.extend(server::sources(&*host, event.player, event.pos));
    }

    /// A recipe-book click at a crafting table beside the chests (with the comfort): the missing
    /// ingredients come from the chests.
    fn player_recipe_book_click(&self, host: &mut dyn ServerHost, event: &mut PlayerRecipeBookClickEvent<'_>) {
        server::recipe_clicked(host, event.player, event.recipe, event.all);
    }

    /// Item frames are tracked as they come and go: a frame on a chest is its label.
    fn entity_join_level(&self, _: &mut dyn ServerHost, event: &mut EntityJoinLevelEvent<'_>) -> Flow {
        server::entity_joined(event.dimension, event.entity, event.kind);
        Flow::Continue
    }

    fn entity_leave_level(&self, _: &mut dyn ServerHost, event: &mut EntityLeaveLevelEvent<'_>) {
        server::entity_left(event.entity, event.kind);
    }

    fn player_container_open(&self, host: &mut dyn ServerHost, event: &mut PlayerContainerOpenEvent<'_>) {
        if let Some(pos) = event.pos {
            server::opened(host, event.player, event.dimension, pos);
        }
    }

    fn player_container_close(&self, host: &mut dyn ServerHost, event: &mut PlayerContainerCloseEvent<'_>) {
        server::closed(host, event.player);
        shulker::closed(host, event.player);
    }

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        server::forget(event.player);
        skills::forget(event.player);
        shulker::forget(event.player);
    }

    /// Furnaces from chests: a furnace that burned fuel, finished an item or was emptied is
    /// looked at on the next tick.
    fn furnace_burn(&self, _: &mut dyn ServerHost, event: &mut FurnaceBurnEvent<'_>) {
        furnace::woke(event.pos);
    }

    fn furnace_smelt(&self, _: &mut dyn ServerHost, event: &mut FurnaceSmeltEvent<'_>) {
        furnace::woke(event.pos);
    }

    fn furnace_extract(&self, _: &mut dyn ServerHost, event: &mut FurnaceExtractEvent<'_>) {
        furnace::woke(event.pos);
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            server::tick(host);
        }
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        client::tick(host);
        shulker::client_tick(host);
    }

    /// Open a shulker box from the bag: a right-click on one box with an empty cursor, on both
    /// sides, before vanilla's half-pickup (see `shulker.rs`).
    fn item_stacked_on_other(&self, context: &dyn mod_api::LoopContext, event: &mut mod_api::ItemStackedOnOtherEvent<'_, '_>) -> Flow {
        if shulker::clicked(context, event) { Flow::Cancel } else { Flow::Continue }
    }

    /// A menu `open_menu` opened closed: an open shulker box goes back where it came from.
    fn menu_closed(&self, host: &mut dyn ServerHost, event: &mut mod_api::MenuClosedEvent) {
        shulker::closed(host, event.uuid);
    }

    fn screen_closing(&self, _: &dyn ClientHost, _: &mut ScreenClosingEvent<'_>) {
        client::closed();
    }

    fn screen_render_background(&self, host: &dyn ClientHost, event: &mut ScreenRenderBackgroundEvent<'_>) {
        client::draw(host, event.draw, event.screen);
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        client::hover(host, event.draw, event.screen);
    }

    fn screen_mouse_button_pressed_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
        if client::press(host, event.screen, event.button, event.modifiers) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if client::scroll(host, event.screen, event.scroll[1], event.modifiers) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_key_pressed_pre(&self, host: &dyn ClientHost, event: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
        if client::key(host, event.screen, event.key, event.modifiers) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_char_typed_pre(&self, host: &dyn ClientHost, event: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
        if client::char_typed(host, event.screen, event.c) { Flow::Cancel } else { Flow::Continue }
    }
}

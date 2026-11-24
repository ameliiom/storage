//! Storage: open any chest joined to others and everything in all of them shows as one list
//! beside it. See `docs/design.md`.
//!
//! - **The chain** (`chain.rs`): which containers a chest is joined to.
//! - **The merged view** (`entries.rs`): one entry per kind, search, sort, splitting a take.
//! - **The server** (`server.rs`): a session per open chest, takes and puts in one transaction.
//! - **The panel** (`client.rs`): beside the chest's own screen, in the game's look.
//! - **Comforts** (`skills.rs`): Find, quick deposit and a bigger network, earned in Skills' tree
//!   Organization through Storage's own attributes and statistic.
pub mod chain;
pub mod craft;
mod client;
pub mod entries;
pub mod payloads;
mod server;
mod skills;

use mod_api::{
    ClientHost, ClientTickPreEvent, Flow, Hooks, IPayloadContext, PayloadId, PlayerContainerCloseEvent,
    PlayerContainerOpenEvent, PlayerLoggedOutEvent, PlayerRecipeBookClickEvent, RecipeSourcesEvent, Registrar, RegistrationContext, ScreenCharTypedPreEvent,
    ScreenClosingEvent, ScreenKeyPressedPreEvent, ScreenMouseButtonPressedPreEvent, ScreenMouseScrolledPreEvent,
    ScreenRenderBackgroundEvent, ScreenRenderPostEvent, ServerHost, ServerTickEvent, ServerTickPhase, SettingSpec,
};
#[allow(unused_imports)]
use mod_api::prelude::*;

pub(crate) const MOD_ID: &str = "storage";

/// Settings (Options > Mods > Storage).
pub(crate) mod setting {
    /// Opens every comfort for a world that doesn't use Skills.
    pub const ALL_COMFORTS: &str = "all_comforts";
}

fn on_view(view: payloads::View, _: &mut dyn IPayloadContext) -> Result<(), String> {
    client::viewed(view);
    Ok(())
}

#[derive(Default)]
pub struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let _ = r.register_setting(SettingSpec::toggle(setting::ALL_COMFORTS, "storage.config.all_comforts", false));
        skills::register(r);
        let mut handlers = r.payload_handlers();
        let mut view = handlers.registrar("1").optional();
        let _ = payloads::VIEW.set(view.play_to_client_typed(PayloadId("storage:view".into()), on_view));
        let _ = payloads::CLICK.set(view.play_to_server_typed(PayloadId("storage:click".into()), server::click));
        let _ = payloads::COMFORT.set(view.play_to_server_typed(PayloadId("storage:comfort".into()), skills::comfort));
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

    fn player_container_open(&self, host: &mut dyn ServerHost, event: &mut PlayerContainerOpenEvent<'_>) {
        if let Some(pos) = event.pos {
            server::opened(host, event.player, event.dimension, pos);
        }
    }

    fn player_container_close(&self, _: &mut dyn ServerHost, event: &mut PlayerContainerCloseEvent<'_>) {
        server::closed(event.player);
    }

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        server::closed(event.player);
    }

    fn server_tick(&self, host: &mut dyn ServerHost, event: &mut ServerTickEvent<'_>) {
        if matches!(event.phase, ServerTickPhase::ServerTickEnd { .. }) {
            server::tick(host);
        }
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        client::tick(host);
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

    fn screen_mouse_scrolled_pre(&self, _: &dyn ClientHost, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if client::scroll(event.screen, event.scroll[1]) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_key_pressed_pre(&self, host: &dyn ClientHost, event: &mut ScreenKeyPressedPreEvent<'_>) -> Flow {
        if client::key(host, event.screen, event.key, event.modifiers) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_char_typed_pre(&self, host: &dyn ClientHost, event: &mut ScreenCharTypedPreEvent<'_>) -> Flow {
        if client::char_typed(host, event.screen, event.c) { Flow::Cancel } else { Flow::Continue }
    }
}

//! Storage: one terminal shows every chest joined to it as one inventory. See `docs/design.md`.
//!
//! - **The chain** (`chain.rs`): which containers a terminal reaches.
//! - **The merged view** (`entries.rs`): one entry per kind, search, sort, splitting a take.
//! - **The server** (`server.rs`): reads the network on a click, moves items in one transaction.
//! - **The screen** (`client.rs`): the creative inventory's look, drawing the server's view.
pub mod chain;
mod client;
pub mod entries;
pub mod payloads;
mod server;

use mod_api::{
    BlockPart, ClientHost, ClientTickPreEvent, Flow, Hooks, IPayloadContext, InteractionResult, PayloadId,
    PlayerLoggedOutEvent, Registrar, RegistrationContext, ScreenMouseButtonPressedPreEvent,
    ScreenMouseScrolledPreEvent, ScreenRenderPostEvent, ServerHost, UseWithoutItemEvent,
};
#[allow(unused_imports)]
use mod_api::prelude::*;

pub(crate) const MOD_ID: &str = "storage";
const TERMINAL: &str = "storage:terminal";

/// The terminal block: an empty-hand click opens the network.
struct Terminal;

#[mod_api::horizon_part]
impl BlockPart for Terminal {
    fn use_without_item(&self, host: &mut dyn ServerHost, event: &mut UseWithoutItemEvent<'_>) -> Option<InteractionResult> {
        let click = event.click;
        server::open(host, click.player, click.dimension.key().id(), click.pos);
        Some(InteractionResult::Success)
    }
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
        let terminal = r.block_key(TERMINAL);
        r.block_part(terminal, Terminal);
        let mut handlers = r.payload_handlers();
        let mut view = handlers.registrar("1").optional();
        let _ = payloads::VIEW.set(view.play_to_client_typed(PayloadId("storage:view".into()), on_view));
        let _ = payloads::TAKE.set(view.play_to_server_typed(PayloadId("storage:take".into()), server::take));
    }

    fn client_tick_pre(&self, host: &mut dyn ClientHost, _: &mut ClientTickPreEvent) {
        client::tick(host);
    }

    fn screen_render_post(&self, host: &dyn ClientHost, event: &mut ScreenRenderPostEvent<'_>) {
        client::draw(host, event.draw, event.screen.mouse);
    }

    fn screen_mouse_button_pressed_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseButtonPressedPreEvent<'_>) -> Flow {
        if client::press(host, event.button, event.screen.mouse) { Flow::Cancel } else { Flow::Continue }
    }

    fn screen_mouse_scrolled_pre(&self, host: &dyn ClientHost, event: &mut ScreenMouseScrolledPreEvent<'_>) -> Flow {
        if client::scroll(host, event.scroll[1]) { Flow::Cancel } else { Flow::Continue }
    }

    fn player_logged_out(&self, _: &mut dyn ServerHost, event: &mut PlayerLoggedOutEvent<'_>) {
        server::left(event.player);
    }
}

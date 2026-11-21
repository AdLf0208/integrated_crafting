//! Integrated Crafting for Horizon slice 1: a Crafting Interface that runs crafting
//! jobs from its own slots.
//!
//! Upstream the interface is a part on an Integrated Dynamics cable: it exposes a
//! recipe to the network, pulls the job's ingredients from the network's storage and
//! inserts them into the machine it faces. **This pin has no cross-mod API**, so a
//! part cannot join another port's network and a recipe cannot come from a variable
//! card. What is faithful is the job itself: the interface resolves a request against
//! the game's crafting recipes, keeps that recipe for the job's life, holds the
//! ingredients, crafts one recipe at a time, blocks with the reason when it cannot
//! continue, and reports the queue. The gap the network leaves is recorded in
//! `notes.md` as `blocked(no-parent-network-attachment-api)`.
//!
//! The rules live in [`runtime`], which has no game access and is unit-tested on its
//! own. This file is the one `impl Hooks` plus the interface's Part; the game calls the
//! Part only for states of `integratedcrafting:part_interface_crafting`, and each
//! method is a thin call into `support`.
pub mod runtime;
mod support;

use std::sync::Arc;

use mod_api::{
    BlockEntityEvent, BlockEntityPart, BlockEntityTickEvent, BlockEntityTicksEvent, BlockPart,
    Compound, Flow, Hooks, Host, LevelSide, RegisterCommandsEvent, Registrar, RegistrationContext,
    RunModCommandEvent, ServerHost,
};

/// The Crafting Interface block (and its item).
pub(crate) const INTERFACE: &str = "integratedcrafting:part_interface_crafting";

/// The one Part attached to the Crafting Interface: its block entity (the saved job
/// queue and the item slots) and the tick that drives the engine.
///
/// A moment that belongs to one block is a Part, not a global hook, so the game asks
/// into this type only for states of `integratedcrafting:part_interface_crafting`.
struct Interface;

/// The interface has no block-level interaction of its own: its ingredient slots are
/// reached through the declared container (a hopper above it, or
/// `/integratedcrafting feed`), and the part's screen is out of this slice.
#[mod_api::horizon_part]
impl BlockPart for Interface {}

#[mod_api::horizon_part]
impl BlockEntityPart for Interface {
    fn block_entity(
        &self,
        _level: LevelSide,
        event: &mut BlockEntityEvent<'_>,
    ) -> Option<Compound> {
        support::on_block_entity(event)
    }

    fn block_entity_ticks(&self, _host: &dyn Host, event: &mut BlockEntityTicksEvent<'_>) {
        // Every state of the interface holds a job queue, so every state ticks.
        event.ticks = true;
    }

    fn block_entity_tick(&self, host: &mut dyn ServerHost, event: &mut BlockEntityTickEvent<'_>) {
        support::on_block_entity_tick(host, event);
    }
}

/// The mod itself holds nothing: the queue lives in the block entity's saved compound,
/// so a Part — a separate object — needs no state of its own.
#[derive(Default)]
struct Mod;

#[mod_api::horizon_mod]
impl Hooks for Mod {
    /// Attach the interface's Part to its block, declare its item slots and the block
    /// entity type the Part owns.
    ///
    /// One attachment, not two: `expand_block_entity_parts` copies a block entity's Part into the
    /// block table with the same name, so an extra `attach_block_part` of the same object is
    /// refused as `interface is attached to … twice` and **crashes the game at startup**
    /// (measured 2026-11-19; the SDK's `parts/block_entities.rs:5-33`).
    fn register(&self, r: &mut dyn Registrar, _cx: &RegistrationContext) {
        let block = r.block_key(INTERFACE);
        r.block_container(&block, support::interface_container());
        let kind = r.declare_block_entity_type("interface", vec![INTERFACE.to_owned()]);
        if let Err(error) = r.attach_block_entity_part(&kind, "interface", Arc::new(Interface)) {
            support::warn_refused(error);
        }
    }

    /// `/integratedcrafting status|request|cancel|feed` (the readout, and the paths a
    /// screen and a network would otherwise provide).
    fn register_commands(&self, host: &mut dyn ServerHost, event: &mut RegisterCommandsEvent<'_>) {
        support::on_register_commands(host, event);
    }

    fn run_mod_command(
        &self,
        host: &mut dyn ServerHost,
        event: &mut RunModCommandEvent<'_>,
    ) -> Flow {
        support::on_run_mod_command(host, event)
    }
}

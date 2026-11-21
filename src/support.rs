// Game-side glue for Integrated Crafting slice 1.
//
// The interface is one block with one block entity: the block entity's saved compound
// holds the job queue, and its declared item container holds the ingredients and the
// results. Every tick rebuilds the pure engine from the compound, advances it once,
// and writes it back; a tick that changes nothing answers `Unchanged`, so an idle
// interface does not dirty its chunk.
//
// The engine never touches the game: this module resolves a request's recipe once, at
// submission (upstream keeps a running job's recipe even when the recipe changes, so a
// job stores its own resolved ingredient cells), converts the block entity's item
// stacks into the engine's counting model and back, and prints the readout.
//
// Unlike upstream, the ingredients are not pulled from an Integrated Dynamics network
// (no cross-mod API exists at this pin): they come from this block's own input slots,
// and the results stay in its output slots. `notes.md` records that gap by name.
use mod_api::{
    BlockEntityEvent, BlockEntityTickEvent, BlockEntityTickReply, Command, CommandArgumentValue,
    CommandInvocation, CommandSource, Compound, ENTITY_ITEMS, Flow, Host, HostTransaction,
    Identifier, ItemResource, ItemStack, LevelRef, ModContainer, PlayerRef, RegisterCommandsEvent,
    RunModCommandEvent, ServerHost, Tag, Text,
};

use crate::INTERFACE;
use crate::runtime::{
    Blocked, DEFAULT_OUTPUT_LIMIT, Engine, Job, JobState, Missing, REACH_BLOCKS, Recipe, Request,
    Sink, Stack, Step, Storage, merge_container, within_reach,
};

/// The ingredient slots: a crafting table's 3×3 grid, in click order.
pub(crate) const INPUT_SLOTS: usize = 9;

/// The result slots. A craft's whole output must fit in them, or the job blocks.
pub(crate) const OUTPUT_SLOTS: usize = 4;

/// Every slot of the interface's container.
pub(crate) const SLOTS: usize = INPUT_SLOTS + OUTPUT_SLOTS;

/// The block entity's saved key for the job list.
const JOBS_TAG: &str = "jobs";
/// The block entity's saved key for the next job's id.
const NEXT_TAG: &str = "next_job";
/// The block entity's saved key for its 13 slots (the one place a command and the tick agree).
/// Not `items`: that name is the block entity's own container storage, and a payload carrying it
/// does not come back through `replace_data`.
const ITEMS_TAG: &str = "ic_slots";
/// The block entity's saved key for what the interface last wrote into its container.
const MIRROR_TAG: &str = "ic_mirror";

/// A placed interface gets a block entity for its job queue
/// (`EntityBlock.newBlockEntity`).
pub(crate) fn on_block_entity(event: &mut BlockEntityEvent<'_>) -> Option<Compound> {
    (event.block == INTERFACE).then(Compound::new)
}

/// The interface's item slots: hoppers above it fill the ingredient slots, hoppers
/// below and beside it take the results, and every face sees the whole container.
///
/// The order is the engine's: slots `0..INPUT_SLOTS` are the ingredients, the rest the
/// results.
pub(crate) fn interface_container() -> ModContainer {
    let all: Vec<usize> = (0..SLOTS).collect();
    let inputs: Vec<usize> = (0..INPUT_SLOTS).collect();
    let outputs: Vec<usize> = (INPUT_SLOTS..SLOTS).collect();
    ModContainer {
        slots: SLOTS,
        // `Direction` order: down, up, north, south, west, east.
        faces: [
            outputs.clone(),
            inputs.clone(),
            all.clone(),
            all.clone(),
            all.clone(),
            all,
        ],
        inputs,
        outputs,
        menu: None,
    }
}

/// A registration the game refused is a mod bug, not a runtime condition.
pub(crate) fn warn_refused(error: impl std::fmt::Display) {
    eprintln!(
        "Integrated Crafting: the interface's block entity part was refused: {error} -- the interface will not run a job"
    );
}

/// One server tick: fold in what a hopper changed, advance the queue, and save the slots back.
///
/// The 13 slots live in the block entity's **saved compound** (`ic_slots`, `ic_mirror`), because
/// that is the one place a command and the tick agree on. Measured in game: a command's
/// `BlockEntityRef::set_item_slot` write never reached the tick's `event.entity.items`, so a feed
/// was wiped on the next tick, and a command's `replace_data` *inside* a transaction was rolled
/// back by it. The tick mirrors the saved slots into the container the hoppers reach and folds a
/// hopper's change back with [`merge_container`] — for the ingredient **and** the result slots,
/// and against the state the interface itself last wrote (the mirror), so a hopper that drains a
/// result is not handed the same stack again every tick.
pub(crate) fn on_block_entity_tick(
    host: &mut dyn ServerHost,
    event: &mut BlockEntityTickEvent<'_>,
) {
    let saved = read_storage(&event.entity.nbt);
    let mirror = read_mirror(&event.entity.nbt);
    let container = storage_of(host, &event.entity.items);
    let merged_inputs = merge_container(&saved.inputs, &mirror.inputs, &container.inputs);
    let merged_outputs = merge_container(&saved.outputs, &mirror.outputs, &container.outputs);
    let mut storage = Storage::new(merged_inputs, merged_outputs);
    let mut engine = read_engine(&Tag::Compound(event.entity.nbt.clone()));
    let events = engine.tick(&mut storage);
    // The container's copy is refreshed from the saved slots whether or not a craft ran, so a
    // hopper never reads a copy the interface no longer believes.
    let container_before = event.entity.items.clone();
    let items = items_of(host, &storage, &container_before);
    let items_changed = items != container_before;
    let mut data = event.entity.nbt.clone();
    save_engine(&mut data, &engine);
    let written: Vec<Option<Stack>> = storage
        .inputs
        .iter()
        .chain(storage.outputs.iter())
        .cloned()
        .collect();
    write_items(&mut data, &storage.inputs, &storage.outputs, &written);
    let data_changed = data != event.entity.nbt;
    if events.is_empty() && !items_changed && !data_changed {
        // A waiting job whose ingredients and slots have not changed yet.
        *event.reply = BlockEntityTickReply::Unchanged;
        return;
    }
    if items_changed {
        event.entity.items = items;
    }
    if data_changed {
        event.entity.nbt = data;
    }
}

/// Read the queue out of a block entity's saved compound; an interface nobody has
/// used yet holds an empty compound and gives an empty queue.
fn read_engine(data: &Tag) -> Engine {
    let Some(Tag::List(jobs)) = data.get(JOBS_TAG) else {
        return Engine::new();
    };
    let jobs: Vec<Job> = jobs.iter().filter_map(read_job).collect();
    let next = int(data, NEXT_TAG)
        .unwrap_or(1)
        .clamp(1, i64::from(i32::MAX)) as u32;
    Engine::from_parts(jobs, next)
}

/// One job from its saved compound; `None` for anything malformed (a job this build
/// cannot read is dropped rather than trusted with the slots).
///
/// The chain is saved as its `steps`; a compound written by 0.2.1 has the single recipe's
/// `cells` at the top level instead, and is read as a one-step job so an upgrade keeps the
/// jobs an interface was already running.
fn read_job(tag: &Tag) -> Option<Job> {
    let steps = match tag.get("steps") {
        Some(Tag::List(steps)) => {
            let steps: Vec<Step> = steps.iter().map(read_step).collect::<Option<Vec<_>>>()?;
            if steps.is_empty() {
                return None;
            }
            steps
        }
        _ => vec![read_legacy_step(tag)?],
    };
    let recipe = steps.last()?.recipe.clone();
    let blocked = match string(tag, "blocked").as_deref() {
        Some("output-full") => Some(Blocked::OutputFull),
        Some("input-full") => Some(Blocked::InputFull),
        Some("missing") => Some(Blocked::MissingIngredients(read_missing(tag))),
        _ => None,
    };
    Some(Job {
        id: count(tag, "id")?,
        recipe,
        output_limit: count(tag, "limit")?.max(1),
        requested: count(tag, "requested")?,
        produced: count(tag, "produced")?,
        steps,
        state: JobState::from_name(&string(tag, "state")?)?,
        blocked,
    })
}

/// One step of a saved chain.
fn read_step(tag: &Tag) -> Option<Step> {
    Some(Step {
        recipe: Recipe::new(
            string(tag, "recipe")?,
            read_cells(tag)?,
            string(tag, "out")?,
            count(tag, "out_count")?,
        ),
        crafts_needed: count(tag, "crafts_needed")?,
        crafts: count(tag, "crafts")?,
        progress: count(tag, "progress")?,
        sink: Sink::from_name(&string(tag, "sink")?)?,
        limit: count(tag, "limit")?.max(1),
    })
}

/// A one-step job as 0.2.1 saved it: the recipe and its progress at the top level of the job.
fn read_legacy_step(tag: &Tag) -> Option<Step> {
    Some(Step {
        recipe: Recipe::new(
            string(tag, "recipe")?,
            read_cells(tag)?,
            string(tag, "out")?,
            count(tag, "out_count")?,
        ),
        crafts_needed: count(tag, "crafts_needed")?,
        crafts: count(tag, "crafts")?,
        progress: count(tag, "progress")?,
        sink: Sink::Results,
        limit: count(tag, "limit")?.max(1),
    })
}

/// A recipe's ingredient cells as saved: a list of lists of item ids. Empty, or a cell that
/// names no item, is not a recipe this engine can run.
fn read_cells(tag: &Tag) -> Option<Vec<Vec<String>>> {
    let Tag::List(cells) = tag.get("cells")? else {
        return None;
    };
    let cells: Vec<Vec<String>> = cells
        .iter()
        .map(|cell| match cell {
            Tag::List(ids) => Some(ids.iter().filter_map(string_of).collect::<Vec<String>>()),
            _ => None,
        })
        .collect::<Option<Vec<Vec<String>>>>()?;
    (!cells.is_empty() && cells.iter().all(|cell| !cell.is_empty())).then_some(cells)
}

/// The saved missing ingredients of a blocked job.
fn read_missing(tag: &Tag) -> Vec<Missing> {
    let Some(Tag::List(missing)) = tag.get("missing") else {
        return Vec::new();
    };
    missing
        .iter()
        .filter_map(|one| {
            let candidates = match one.get("candidates") {
                Some(Tag::List(ids)) => ids.iter().filter_map(string_of).collect(),
                _ => return None,
            };
            Some(Missing {
                candidates,
                needed: count(one, "needed").unwrap_or(0),
                have: count(one, "have").unwrap_or(0),
            })
        })
        .collect()
}

/// Write the queue into the block entity's saved compound, leaving every other key alone: a
/// payload that replaced the whole compound would drop the interface's slots, which live beside
/// the queue (measured in game: the request after a feed wiped the items).
fn save_engine(data: &mut Compound, engine: &Engine) {
    put(
        data,
        JOBS_TAG,
        Tag::List(engine.jobs().iter().map(write_job).collect()),
    );
    put(data, NEXT_TAG, Tag::Int(engine.next_id() as i32));
}

/// One job as the block entity saves it.
fn write_job(job: &Job) -> Tag {
    let mut data = Compound::new();
    data.insert("id".into(), Tag::Int(job.id as i32));
    data.insert("recipe".into(), Tag::String(job.recipe.id.as_str().into()));
    data.insert("out".into(), Tag::String(job.recipe.output.as_str().into()));
    data.insert("out_count".into(), Tag::Int(job.recipe.output_count as i32));
    data.insert("limit".into(), Tag::Int(job.output_limit as i32));
    data.insert("requested".into(), Tag::Int(job.requested as i32));
    data.insert("produced".into(), Tag::Int(job.produced as i32));
    data.insert("state".into(), Tag::String(job.state.name().into()));
    data.insert(
        "steps".into(),
        Tag::List(job.steps.iter().map(write_step).collect()),
    );
    match &job.blocked {
        Some(Blocked::OutputFull) => {
            data.insert("blocked".into(), Tag::String("output-full".into()));
        }
        Some(Blocked::InputFull) => {
            data.insert("blocked".into(), Tag::String("input-full".into()));
        }
        Some(Blocked::MissingIngredients(missing)) => {
            data.insert("blocked".into(), Tag::String("missing".into()));
            let entries = missing
                .iter()
                .map(|one| {
                    let mut entry = Compound::new();
                    let ids = one
                        .candidates
                        .iter()
                        .map(|id| Tag::String(id.as_str().into()))
                        .collect();
                    entry.insert("candidates".into(), Tag::List(ids));
                    entry.insert("needed".into(), Tag::Int(one.needed as i32));
                    entry.insert("have".into(), Tag::Int(one.have as i32));
                    Tag::Compound(entry)
                })
                .collect();
            data.insert("missing".into(), Tag::List(entries));
        }
        None => {}
    }
    Tag::Compound(data)
}

/// One step of a job's chain as the block entity saves it.
fn write_step(step: &Step) -> Tag {
    let mut data = Compound::new();
    data.insert("recipe".into(), Tag::String(step.recipe.id.as_str().into()));
    data.insert(
        "out".into(),
        Tag::String(step.recipe.output.as_str().into()),
    );
    data.insert(
        "out_count".into(),
        Tag::Int(step.recipe.output_count as i32),
    );
    data.insert("limit".into(), Tag::Int(step.limit as i32));
    data.insert("crafts_needed".into(), Tag::Int(step.crafts_needed as i32));
    data.insert("crafts".into(), Tag::Int(step.crafts as i32));
    data.insert("progress".into(), Tag::Int(step.progress as i32));
    data.insert("sink".into(), Tag::String(step.sink.name().into()));
    let cells = step
        .recipe
        .ingredients
        .iter()
        .map(|cell| {
            Tag::List(
                cell.iter()
                    .map(|id| Tag::String(id.as_str().into()))
                    .collect(),
            )
        })
        .collect();
    data.insert("cells".into(), Tag::List(cells));
    Tag::Compound(data)
}

/// A `String` tag's text.
fn string_of(tag: &Tag) -> Option<String> {
    match tag {
        Tag::String(text) => Some(text.to_string_lossy()),
        _ => None,
    }
}

/// A text field of a compound.
fn string(tag: &Tag, key: &str) -> Option<String> {
    tag.get(key).and_then(string_of)
}

/// An integer field of a compound, as an unsigned count.
fn count(tag: &Tag, key: &str) -> Option<u32> {
    let value = int(tag, key)?;
    u32::try_from(value).ok()
}

/// An integer field of a compound.
fn int(tag: &Tag, key: &str) -> Option<i64> {
    match tag.get(key) {
        Some(Tag::Int(value)) => Some(i64::from(*value)),
        Some(Tag::Long(value)) => Some(*value),
        Some(Tag::Short(value)) => Some(i64::from(*value)),
        Some(Tag::Byte(value)) => Some(i64::from(*value)),
        _ => None,
    }
}

/// The interface's slots as the engine's counting model: the first
/// [`INPUT_SLOTS`] are the ingredients, the rest the results.
fn storage_of(host: &dyn Host, items: &[Option<ItemStack>]) -> Storage {
    let model = |slot: &Option<ItemStack>| {
        slot.as_ref()
            .map(|stack| Stack::new(stack.item().id(host), stack.count()))
    };
    let split = INPUT_SLOTS.min(items.len());
    let inputs = items[..split].iter().map(model).collect();
    let outputs = items[split..].iter().map(model).collect();
    Storage::new(inputs, outputs)
}

/// The slots after the engine ran, keeping every stack's components: a stack the
/// engine only counted keeps its own item data and takes the new count.
fn items_of(
    host: &dyn Host,
    storage: &Storage,
    original: &[Option<ItemStack>],
) -> Vec<Option<ItemStack>> {
    let mut items = Vec::with_capacity(SLOTS);
    let model = storage.inputs.iter().chain(storage.outputs.iter());
    for (slot, wanted) in model.enumerate() {
        items.push(rebuild(host, wanted, original.get(slot)));
    }
    items
}

/// One slot's stack: the original stack when the item is the same, else a fresh one.
fn rebuild(
    host: &dyn Host,
    wanted: &Option<Stack>,
    original: Option<&Option<ItemStack>>,
) -> Option<ItemStack> {
    let wanted = wanted.as_ref()?;
    if let Some(Some(original)) = original
        && original.item().id(host) == wanted.item
    {
        let mut stack = original.clone();
        let _ = stack.set_count(wanted.count);
        return Some(stack);
    }
    let key = host.item_key(&Identifier::parse_namespaced(&wanted.item)?)?;
    ItemStack::new(key, wanted.count)
}

/// Declare `/integratedcrafting status|request|cancel|feed`.
pub(crate) fn on_register_commands(
    _host: &mut dyn ServerHost,
    event: &mut RegisterCommandsEvent<'_>,
) {
    event
        .commands
        .push(Command::new("integratedcrafting status <pos:block_pos>"));
    event.commands.push(Command::new(
        "integratedcrafting request <pos:block_pos> <item:item> <count:integer>",
    ));
    event
        .commands
        .push(Command::new("integratedcrafting cancel <pos:block_pos>"));
    event.commands.push(Command::new(
        "integratedcrafting feed <pos:block_pos> <item:item> <count:integer>",
    ));
}

/// Run `/integratedcrafting status <x> <y> <z>` (the readout), `request <x> <y> <z>
/// <item> <count>` (submit a crafting job), `cancel <x> <y> <z>` (drop every job this
/// interface still holds) or `feed <x> <y> <z> <item> <count>` (put items into the
/// input slots as a hopper above the interface does; the part's own screen is out of
/// this slice).
pub(crate) fn on_run_mod_command(
    host: &mut dyn ServerHost,
    event: &mut RunModCommandEvent<'_>,
) -> Flow {
    let invocation = event.command;
    let mut literals = invocation.literals.iter().map(String::as_str);
    if literals.next() != Some("integratedcrafting") {
        return Flow::Continue;
    }
    let Some(subcommand) = literals.next() else {
        return Flow::Continue;
    };
    let Some(dimension) = invocation.dimension.id(host) else {
        reply(host, "Integrated Crafting: that dimension is not known.");
        return Flow::Cancel;
    };
    let Some(level) = host.level(&dimension) else {
        reply(host, "Integrated Crafting: that dimension is not loaded.");
        return Flow::Cancel;
    };
    let Some(pos) = block_pos(invocation, "pos") else {
        reply(host, "Usage: /integratedcrafting <subcommand> <x> <y> <z>");
        return Flow::Cancel;
    };
    // `status` is a readout of this mod's own block; the three that change an interface act on it
    // as an ordinary interaction does, so a player must be within reach of it. The console, a
    // command block and a function are the server's own hand and act anywhere (and `feed` from
    // one of those has no inventory to take from, so it is refused there for that reason).
    let player = match event.source {
        CommandSource::Player { player, .. } => Some(*player),
        _ => None,
    };
    if matches!(subcommand, "request" | "cancel" | "feed")
        && let Some(player) = player
        && !player
            .entity()
            .position(&*host)
            .is_some_and(|at| within_reach(at, pos, REACH_BLOCKS))
    {
        reply(
            host,
            &format!("Integrated Crafting: stand within {REACH_BLOCKS} blocks of the interface."),
        );
        return Flow::Cancel;
    }
    match subcommand {
        "status" => status(host, level, pos),
        "request" => request(host, level, pos, invocation),
        "cancel" => cancel(host, level, pos),
        "feed" => feed(host, level, pos, invocation, player),
        _ => reply(
            host,
            "Usage: /integratedcrafting status|request|cancel|feed <x> <y> <z> ...",
        ),
    }
    Flow::Cancel
}

/// `/integratedcrafting status`: the queue's readout, byte for byte the engine's.
fn status(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) {
    let Some(entity) = interface_at(host, level, pos) else {
        reply(host, "Integrated Crafting: no crafting interface there.");
        return;
    };
    let engine = read_engine(&Tag::Compound(entity.data(&*host).unwrap_or_default()));
    for line in engine.readout(pos) {
        reply(host, &line);
    }
}

/// `/integratedcrafting request`: resolve the item's crafting recipe and queue a job.
fn request(
    host: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    invocation: &CommandInvocation,
) {
    let Some(entity) = interface_at(host, level, pos) else {
        reply(host, "Integrated Crafting: no crafting interface there.");
        return;
    };
    let Some(item) = item_arg(invocation, "item") else {
        reply(
            host,
            "Usage: /integratedcrafting request <x> <y> <z> <item> <count>",
        );
        return;
    };
    let Some(wanted) = count_arg(invocation, "count") else {
        reply(
            host,
            "Integrated Crafting: the count must be a positive number.",
        );
        return;
    };
    let output = item.item().id(&*host);
    let table = crafting_table(&*host);
    if Engine::recipe_for(&table, &output).is_none() {
        reply(
            host,
            &format!("Integrated Crafting: no crafting recipe makes {output}"),
        );
        return;
    }
    let limit = host
        .max_stack_size(item.item())
        .unwrap_or(DEFAULT_OUTPUT_LIMIT);
    let data = entity.data(&*host).unwrap_or_default();
    let storage = read_storage(&data);
    let mut engine = read_engine(&Tag::Compound(data.clone()));
    match engine.submit(
        &table,
        &storage,
        &Request::new(output.clone(), wanted),
        limit,
    ) {
        Ok(id) => {
            // A chain names every step, so the acceptance reply says what the interface is going
            // to make first; a plain request keeps the line it always had.
            let plan = engine
                .jobs()
                .iter()
                .find(|job| job.id == id)
                .filter(|job| job.steps.len() > 1)
                .map(|job| {
                    job.steps
                        .iter()
                        .map(|step| format!("{}x {}", step.crafts_needed, step.recipe.id))
                        .collect::<Vec<_>>()
                        .join(", ")
                });
            let line = match plan {
                Some(plan) => format!(
                    "Integrated Crafting: job {id} accepted for {wanted} x {output} (crafts: {plan})"
                ),
                None => format!("Integrated Crafting: job {id} accepted for {wanted} x {output}"),
            };
            reply(host, &line);
        }
        Err(refusal) => {
            reply(
                host,
                &format!("Integrated Crafting: {}", refusal.line(&output)),
            );
            return;
        }
    }
    save(host, entity, &engine);
}

/// `/integratedcrafting cancel`: drop every job and report how many were dropped.
fn cancel(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3]) {
    let Some(entity) = interface_at(host, level, pos) else {
        reply(host, "Integrated Crafting: no crafting interface there.");
        return;
    };
    let mut engine = read_engine(&Tag::Compound(entity.data(&*host).unwrap_or_default()));
    let dropped = engine.cancel();
    save(host, entity, &engine);
    reply(
        host,
        &format!("Integrated Crafting: {dropped} job(s) cancelled"),
    );
}

/// `/integratedcrafting feed`: move items from the acting player's inventory into the input slots,
/// as a hopper above the interface does — the items are the player's, and nothing is created. The
/// inventory extraction and the slot writes share one transaction, so a slot the game refuses
/// never costs the player an item, and an item that left the inventory always lands in a slot.
fn feed(
    host: &mut dyn ServerHost,
    level: LevelRef,
    pos: [i32; 3],
    invocation: &CommandInvocation,
    player: Option<PlayerRef>,
) {
    let Some(entity) = interface_at(host, level, pos) else {
        reply(host, "Integrated Crafting: no crafting interface there.");
        return;
    };
    let Some(item) = item_arg(invocation, "item") else {
        reply(
            host,
            "Usage: /integratedcrafting feed <x> <y> <z> <item> <count>",
        );
        return;
    };
    let Some(wanted) = count_arg(invocation, "count") else {
        reply(
            host,
            "Integrated Crafting: the count must be a positive number.",
        );
        return;
    };
    let Some(player) = player else {
        reply(
            host,
            "Integrated Crafting: /feed moves items out of the player's inventory, so it needs a player.",
        );
        return;
    };
    let key = item.item();
    let resource = ItemResource::of(item);
    let data = entity.data(&*host).unwrap_or_default();
    let saved = read_storage(&data);
    let (id, limit) = {
        let read: &dyn Host = &*host;
        (
            key.id(read),
            read.max_stack_size(key).unwrap_or(DEFAULT_OUTPUT_LIMIT),
        )
    };
    let model = saved.inputs.clone();
    let Some(source) = player.entity().capability(host, &ENTITY_ITEMS, ()) else {
        reply(
            host,
            "Integrated Crafting: that player's inventory is not reachable.",
        );
        return;
    };
    let mut available = 0u32;
    for index in 0..source.size(host) {
        if source
            .resource(host, index)
            .is_some_and(|held| held.item() == key)
        {
            available += source.amount(host, index);
        }
    }
    if available == 0 {
        reply(
            host,
            &format!("Integrated Crafting: the player holds no {id} to feed."),
        );
        return;
    }
    // The player's own items are the cost: never more than they hold.
    let wanted = wanted.min(available);
    let (planned, inserted) = plan_insert(&model, &id, wanted, limit);
    if inserted == 0 {
        reply(
            host,
            "Integrated Crafting: the input slots refused the items.",
        );
        return;
    }
    let mut after = data.clone();
    // The feed changes the saved slots only. The mirror stays what the interface last wrote into
    // the container, so the next tick sees that container as unchanged and keeps the fed items.
    write_slots(&mut after, &planned, &saved.outputs);
    // The player is charged first, then the slots are written: a `replace_data` that followed a
    // transaction on the same block entity was rolled back by it at this pin (measured in game),
    // and charging first means a refused write hands the items back instead of duplicating them.
    let taken = host
        .transaction(|host, tx| {
            let mut taken = 0;
            for index in 0..source.size(host) {
                if taken == inserted {
                    break;
                }
                let Some(held) = source.resource(host, index) else {
                    continue;
                };
                if held.item() != key {
                    continue;
                }
                taken += source.extract(host, tx, index, &resource, inserted - taken);
            }
            // Not enough in the inventory: nothing moves and nothing is created.
            Ok::<u32, ()>(taken)
        })
        .unwrap_or(0);
    if taken != inserted {
        let _ = host.transaction(|host, tx| {
            let mut left = taken;
            for index in 0..source.size(host) {
                if left == 0 {
                    break;
                }
                if source
                    .resource(host, index)
                    .is_some_and(|held| held.item() == key)
                {
                    left -= source.insert(host, tx, index, &resource, left);
                }
            }
            Ok::<(), ()>(())
        });
        reply(
            host,
            "Integrated Crafting: the player does not hold that many.",
        );
        return;
    }
    let written = entity.replace_data(host, after).is_ok();
    if !written {
        // Hand the items back: the slots refused the write, so nothing may be kept.
        let _ = host.transaction(|host, tx| {
            let mut left = inserted;
            for index in 0..source.size(host) {
                if left == 0 {
                    break;
                }
                if source
                    .resource(host, index)
                    .is_some_and(|held| held.item() == key)
                {
                    left -= source.insert(host, tx, index, &resource, left);
                }
            }
            Ok::<(), ()>(())
        });
    }
    let outcome = written.then_some(()).ok_or(());
    match outcome {
        Ok(()) => reply(
            host,
            &format!("Integrated Crafting: inserted {inserted} of {wanted}"),
        ),
        Err(_) => reply(
            host,
            "Integrated Crafting: the interface refused the items.",
        ),
    }
}

/// The interface's 13 slots as its saved compound holds them: the first [`INPUT_SLOTS`] are the
/// ingredients, the rest the results.
fn read_storage(data: &Compound) -> Storage {
    split_slots(read_slots(data, ITEMS_TAG))
}

/// What the interface last wrote into its container, in the same layout.
fn read_mirror(data: &Compound) -> Storage {
    split_slots(read_slots(data, MIRROR_TAG))
}

/// `SLOTS` saved slots split into the engine's ingredient and result lists.
fn split_slots(all: Vec<Option<Stack>>) -> Storage {
    let mut inputs: Vec<Option<Stack>> = all.iter().take(INPUT_SLOTS).cloned().collect();
    inputs.resize(INPUT_SLOTS, None);
    let mut outputs: Vec<Option<Stack>> = all
        .into_iter()
        .skip(INPUT_SLOTS)
        .take(OUTPUT_SLOTS)
        .collect();
    outputs.resize(OUTPUT_SLOTS, None);
    Storage::new(inputs, outputs)
}

/// One slot list of a saved compound, `SLOTS` entries long: a list of compounds, each holding
/// `item` and `count`; an empty slot is a compound without them. A shorter or longer list is
/// padded or cut, so a damaged save behaves as empty slots rather than shifting the grid.
fn read_slots(data: &Compound, key: &str) -> Vec<Option<Stack>> {
    let mut slots: Vec<Option<Stack>> = Vec::with_capacity(SLOTS);
    if let Some(Tag::List(list)) = field(data, key) {
        for entry in list.iter() {
            slots.push(match entry {
                Tag::Compound(fields) => field_string(fields, "item")
                    .zip(field_count(fields, "count"))
                    .filter(|(_, count)| *count > 0)
                    .map(|(item, count)| Stack::new(item, count)),
                _ => None,
            });
        }
    }
    slots.resize(SLOTS, None);
    slots.truncate(SLOTS);
    slots
}

/// A compound's field, by name. The key type is UTF-16 and the API does not export it, so a key
/// is taken from `Tag::String`'s own payload (the same conversion a mod's NBT string uses, with no
/// name needed) — the shape moderndynamics' save code uses.
fn field<'a>(data: &'a Compound, name: &str) -> Option<&'a Tag> {
    let Tag::String(key) = Tag::String(name.into()) else {
        return None;
    };
    data.get(&key)
}

/// Insert `value` into a compound under `name`.
fn put(data: &mut Compound, name: &str, value: Tag) {
    if let Tag::String(key) = Tag::String(name.into()) {
        data.insert(key, value);
    }
}

/// A text field of a nested compound.
fn field_string(fields: &Compound, key: &str) -> Option<String> {
    string_of(field(fields, key)?)
}

/// An unsigned integer field of a nested compound.
fn field_count(fields: &Compound, key: &str) -> Option<u32> {
    match field(fields, key) {
        Some(Tag::Int(value)) => u32::try_from(*value).ok(),
        Some(Tag::Long(value)) => u32::try_from(*value).ok(),
        _ => None,
    }
}

/// Write the interface's slots into its saved compound, and the mirror of the state they were
/// written to.
///
/// The mirror is **what the interface last wrote into its container**, not what the container
/// happened to show before the write. A hopper that takes a result then differs from the mirror and
/// wins, so the saved slots lose the item; a mirror holding the pre-write state instead re-inserts
/// the same stack every tick and a hopper under the interface duplicates the job's output (measured
/// in game: an eight-plank job left twelve planks in the chest, and a one-interface craft left four
/// interfaces — fixed in 0.2.2, pinned by the exact chest counts in `tools/e2e.sh` phases B, G, I).
fn write_items(
    data: &mut Compound,
    inputs: &[Option<Stack>],
    outputs: &[Option<Stack>],
    written: &[Option<Stack>],
) {
    write_slots(data, inputs, outputs);
    let mirror: Vec<Tag> = written.iter().map(slot_tag).collect();
    put(data, MIRROR_TAG, Tag::List(mirror));
}

/// Write only the interface's slots, leaving the container mirror as it stands: what a command
/// that changes the saved slots (a feed) does, because it does not touch the container itself.
fn write_slots(data: &mut Compound, inputs: &[Option<Stack>], outputs: &[Option<Stack>]) {
    let slots: Vec<Tag> = inputs.iter().chain(outputs.iter()).map(slot_tag).collect();
    put(data, ITEMS_TAG, Tag::List(slots));
}

/// One slot as its saved compound: `item` and `count`, or an empty compound.
fn slot_tag(slot: &Option<Stack>) -> Tag {
    let Some(stack) = slot else {
        return Tag::Compound(Compound::new());
    };
    let mut fields = Compound::new();
    put(&mut fields, "item", Tag::String(stack.item.as_str().into()));
    put(&mut fields, "count", Tag::Int(stack.count as i32));
    Tag::Compound(fields)
}

/// Every `minecraft:crafting` recipe the game has, with each ingredient cell as its item ids.
///
/// The engine needs the whole table, not just the requested output's recipes: an ingredient cell
/// can name any item of a tag, and any of those may itself be craftable, so the dependency planner
/// asks this table for a recipe of a cell's candidate. It is built once per request (a command,
/// never a tick). A recipe's cells are deduplicated and sorted so the saved job and its readout do
/// not depend on a set's iteration order. `Ingredient::item_keys` is the exact,
/// component-independent membership; a custom predicate it cannot enumerate falls back to the
/// ingredient's display order, which a job then matches by item id alone.
fn crafting_table(host: &dyn Host) -> Vec<Recipe> {
    let mut table = Vec::new();
    for recipe in host.recipes_of_type("minecraft:crafting").iter() {
        let Some(result) = recipe.results.first() else {
            continue;
        };
        if recipe.ingredients.is_empty() {
            continue;
        }
        let mut cells = Vec::with_capacity(recipe.ingredients.len());
        for ingredient in &recipe.ingredients {
            let mut ids: Vec<String> = match ingredient.item_keys(host) {
                Ok(keys) => keys.iter().map(|key| key.id(host)).collect(),
                Err(_) => ingredient.order.clone(),
            };
            ids.sort_unstable();
            ids.dedup();
            if ids.is_empty() {
                cells.clear();
                break;
            }
            cells.push(ids);
        }
        if cells.is_empty() {
            continue;
        }
        table.push(Recipe::new(
            recipe.id.clone(),
            cells,
            result.item().id(host),
            result.count(),
        ));
    }
    table
}

/// Insert `count` of `item` into the ingredient slots: existing stacks of it first, then
/// empty slots. The slots after the insert and how many went in are returned.
fn plan_insert(
    inputs: &[Option<Stack>],
    item: &str,
    count: u32,
    limit: u32,
) -> (Vec<Option<Stack>>, u32) {
    let mut planned = inputs.to_vec();
    let mut remaining = count;
    for slot in &mut planned {
        let Some(stack) = slot else {
            continue;
        };
        if stack.item != item || stack.count >= limit {
            continue;
        }
        let add = (limit - stack.count).min(remaining);
        stack.count += add;
        remaining -= add;
        if remaining == 0 {
            return (planned, count);
        }
    }
    while remaining > 0 {
        let Some(empty) = planned.iter().position(Option::is_none) else {
            break;
        };
        let add = limit.min(remaining);
        planned[empty] = Some(Stack::new(item.to_owned(), add));
        remaining -= add;
    }
    (planned, count - remaining)
}

/// The queue written back into the block entity's saved compound, beside its slots.
fn save(host: &mut dyn ServerHost, entity: mod_api::BlockEntityRef, engine: &Engine) {
    let mut data = entity.data(&*host).unwrap_or_default();
    save_engine(&mut data, engine);
    if let Err(error) = entity.replace_data(host, data) {
        reply(
            host,
            &format!("Integrated Crafting: the interface refused its job data ({error:?})"),
        );
    }
}

/// The block entity of the interface at `pos`, or `None` for another block.
fn interface_at(
    host: &dyn Host,
    level: LevelRef,
    pos: [i32; 3],
) -> Option<mod_api::BlockEntityRef> {
    let block = level.block(host, pos)?;
    if block.id(host) != INTERFACE {
        return None;
    }
    host.block_entity(level, pos)
}

/// An `item` argument.
fn item_arg<'a>(invocation: &'a CommandInvocation, name: &str) -> Option<&'a ItemStack> {
    match invocation.argument(name) {
        Some(CommandArgumentValue::Item(stack)) => Some(stack),
        _ => None,
    }
}

/// A positive `integer` argument.
fn count_arg(invocation: &CommandInvocation, name: &str) -> Option<u32> {
    match invocation.argument(name) {
        Some(CommandArgumentValue::Integer(value)) => u32::try_from(*value).ok().filter(|n| *n > 0),
        _ => None,
    }
}

/// A `block_pos` argument by name.
fn block_pos(invocation: &CommandInvocation, name: &str) -> Option<[i32; 3]> {
    match invocation.argument(name) {
        Some(CommandArgumentValue::BlockPos(pos)) => Some(*pos),
        _ => None,
    }
}

/// Answer a command's source, as the console or chat would.
fn reply(host: &mut dyn ServerHost, line: &str) {
    let _ = host.commands().feedback(Text::literal(line));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saved_slots_round_trip() {
        let mut data = Compound::new();
        let inputs = vec![
            Some(Stack::new("minecraft:oak_log", 2)),
            None,
            Some(Stack::new("minecraft:stick", 7)),
            None,
            None,
            None,
            None,
            None,
            None,
        ];
        let outputs = vec![
            Some(Stack::new("minecraft:oak_planks", 4)),
            None,
            None,
            None,
        ];
        write_items(&mut data, &inputs, &outputs, &[]);
        let storage = read_storage(&data);
        assert_eq!(storage.inputs, inputs, "the ingredients come back");
        assert_eq!(storage.outputs, outputs, "the results come back");
        assert_eq!(
            read_mirror(&data).inputs,
            vec![None; INPUT_SLOTS],
            "the mirror records what the container showed: nothing, here"
        );
        // An untouched compound is 13 empty slots, not a short list.
        assert_eq!(read_storage(&Compound::new()).inputs.len(), INPUT_SLOTS);
        assert_eq!(read_storage(&Compound::new()).outputs.len(), OUTPUT_SLOTS);
    }

    #[test]
    fn a_saved_queue_reads_back_as_it_was_written() {
        let mut engine = Engine::new();
        let storage = Storage::new(
            vec![Some(Stack::new("minecraft:oak_log", 2))],
            vec![None, None],
        );
        let recipe = Recipe::new(
            "minecraft:oak_planks",
            vec![vec!["minecraft:oak_log".to_owned()]],
            "minecraft:oak_planks",
            4,
        );
        engine
            .submit(
                &[recipe],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("two logs pay for eight planks");
        let mut data = Compound::new();
        save_engine(&mut data, &engine);
        let saved = Tag::Compound(data);
        assert_eq!(read_engine(&saved), engine);
    }

    #[test]
    fn a_saved_blocked_reason_reads_back_as_it_was_written() {
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            vec![Some(Stack::new("minecraft:oak_log", 1))],
            vec![Some(Stack::new("minecraft:stick", 64))],
        );
        let recipe = Recipe::new(
            "minecraft:oak_planks",
            vec![vec!["minecraft:oak_log".to_owned()]],
            "minecraft:oak_planks",
            4,
        );
        engine
            .submit(
                &[recipe],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("the ingredients are there");
        for _ in 0..crate::runtime::TICKS_PER_CRAFT {
            engine.tick(&mut storage);
        }
        assert_eq!(engine.jobs()[0].state, JobState::Blocked);
        let mut data = Compound::new();
        save_engine(&mut data, &engine);
        let saved = Tag::Compound(data);
        let restored = read_engine(&saved);
        assert_eq!(restored, engine);
        assert_eq!(
            restored.jobs()[0].blocked,
            Some(Blocked::OutputFull),
            "the reason survives a save"
        );
    }

    #[test]
    fn an_interface_nobody_used_has_an_empty_queue() {
        let data = Tag::Compound(Compound::new());
        assert!(read_engine(&data).is_empty());
    }

    #[test]
    fn a_job_saved_by_the_previous_package_reads_as_one_step() {
        // 0.2.1 saved a job's single recipe and its progress at the top level of the job compound.
        // An interface that is mid-job when the package is upgraded must keep it.
        let mut job = Compound::new();
        job.insert("id".into(), Tag::Int(3));
        job.insert("recipe".into(), Tag::String("minecraft:oak_planks".into()));
        job.insert("out".into(), Tag::String("minecraft:oak_planks".into()));
        job.insert("out_count".into(), Tag::Int(4));
        job.insert("limit".into(), Tag::Int(64));
        job.insert("requested".into(), Tag::Int(8));
        job.insert("produced".into(), Tag::Int(0));
        job.insert("crafts".into(), Tag::Int(1));
        job.insert("crafts_needed".into(), Tag::Int(2));
        job.insert("progress".into(), Tag::Int(5));
        job.insert("state".into(), Tag::String("running".into()));
        job.insert(
            "cells".into(),
            Tag::List(vec![Tag::List(vec![Tag::String(
                "minecraft:oak_log".into(),
            )])]),
        );
        let mut data = Compound::new();
        data.insert(JOBS_TAG.into(), Tag::List(vec![Tag::Compound(job)]));
        let engine = read_engine(&Tag::Compound(data));
        assert_eq!(engine.jobs().len(), 1, "the old job was not dropped");
        let job = &engine.jobs()[0];
        assert_eq!(job.state, JobState::Running);
        assert_eq!(job.steps.len(), 1);
        assert_eq!(job.recipe.output, "minecraft:oak_planks");
        assert_eq!(job.steps[0].sink, Sink::Results);
        assert_eq!(job.steps[0].crafts, 1);
        assert_eq!(job.steps[0].progress, 5);
        assert_eq!(job.steps[0].crafts_needed, 2);
    }

    #[test]
    fn a_saved_chain_reads_back_with_its_steps() {
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            vec![Some(Stack::new("minecraft:oak_log", 1))],
            vec![None, None],
        );
        let planks = Recipe::new(
            "minecraft:oak_planks",
            vec![vec!["minecraft:oak_log".to_owned()]],
            "minecraft:oak_planks",
            4,
        );
        let sticks = Recipe::new(
            "minecraft:stick",
            vec![
                vec!["minecraft:oak_planks".to_owned()],
                vec!["minecraft:oak_planks".to_owned()],
            ],
            "minecraft:stick",
            4,
        );
        engine
            .submit(
                &[planks, sticks],
                &storage,
                &Request::new("minecraft:stick", 4),
                64,
            )
            .expect("one log pays for the chain");
        engine.tick(&mut storage);
        let mut data = Compound::new();
        save_engine(&mut data, &engine);
        let restored = read_engine(&Tag::Compound(data));
        assert_eq!(
            restored, engine,
            "the chain and its progress survive a save"
        );
        assert_eq!(restored.jobs()[0].steps.len(), 2);
        assert_eq!(restored.jobs()[0].steps[0].sink, Sink::Inputs);
        assert_eq!(restored.jobs()[0].steps[1].sink, Sink::Results);
        assert!(
            restored.readout([0, 0, 0])[1].contains("step 1/2"),
            "the restored readout still names the step"
        );
    }

    #[test]
    fn a_mirror_of_the_written_state_lets_a_hopper_take_a_result() {
        // The tick writes the slots and remembers the state it wrote. A hopper that takes one of
        // the results then differs from the mirror and wins (the item has left); an emptied slot
        // wins too. With a mirror of the *pre-write* container instead, the saved stack came back
        // every tick and a hopper under the interface duplicated the job's output (measured in
        // game: 12 planks for an eight-plank job before this fix).
        let saved = vec![Some(Stack::new("minecraft:oak_planks", 8))];
        let written = saved.clone();
        assert_eq!(
            merge_container(
                &saved,
                &written,
                &[Some(Stack::new("minecraft:oak_planks", 7))]
            ),
            vec![Some(Stack::new("minecraft:oak_planks", 7))]
        );
        assert_eq!(merge_container(&saved, &written, &[]), vec![None]);
        // The feed changes only the saved slots; the mirror stays the container's last state, so a
        // container nothing touched keeps the fed items.
        let fed = vec![Some(Stack::new("minecraft:oak_log", 2))];
        let last_written = vec![None];
        assert_eq!(merge_container(&fed, &last_written, &[]), fed);
    }

    #[test]
    fn a_malformed_job_is_dropped_rather_than_trusted() {
        let mut job = Compound::new();
        job.insert("id".into(), Tag::Int(1));
        job.insert("state".into(), Tag::String("running".into()));
        let mut data = Compound::new();
        data.insert(JOBS_TAG.into(), Tag::List(vec![Tag::Compound(job)]));
        assert!(
            read_engine(&Tag::Compound(data)).is_empty(),
            "a job without its recipe cannot be run"
        );
    }

    #[test]
    fn inserting_stacks_into_the_ingredient_slots_fills_and_overflows() {
        let slots = vec![Some(Stack::new("minecraft:oak_log", 60)), None, None];
        let (planned, inserted) = plan_insert(&slots, "minecraft:oak_log", 10, 64);
        assert_eq!(inserted, 10);
        assert_eq!(planned[0], Some(Stack::new("minecraft:oak_log", 64)));
        assert_eq!(planned[1], Some(Stack::new("minecraft:oak_log", 6)));

        let (planned, inserted) = plan_insert(&slots, "minecraft:oak_log", 100, 64);
        assert_eq!(inserted, 100, "three slots hold 4 + 64 + 64");
        assert_eq!(planned[0], Some(Stack::new("minecraft:oak_log", 64)));
        assert_eq!(planned[1], Some(Stack::new("minecraft:oak_log", 64)));
        assert_eq!(planned[2], Some(Stack::new("minecraft:oak_log", 32)));

        // Two slots only: 4 + 64 = 68 fit, so the rest cannot go in.
        let tight = vec![Some(Stack::new("minecraft:oak_log", 60)), None];
        let (planned, inserted) = plan_insert(&tight, "minecraft:oak_log", 100, 64);
        assert_eq!(inserted, 68);
        assert_eq!(planned[0], Some(Stack::new("minecraft:oak_log", 64)));
        assert_eq!(planned[1], Some(Stack::new("minecraft:oak_log", 64)));

        let (planned, inserted) = plan_insert(&slots, "minecraft:stone", 3, 64);
        assert_eq!(inserted, 3);
        assert_eq!(planned[1], Some(Stack::new("minecraft:stone", 3)));
    }
}

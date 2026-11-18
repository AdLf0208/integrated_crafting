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
    CommandInvocation, Compound, Flow, Host, HostTransaction, Identifier, ItemStack, LevelRef,
    ModContainer, RegisterCommandsEvent, RunModCommandEvent, ServerHost, Tag, Text,
};

use crate::INTERFACE;
use crate::runtime::{
    Blocked, DEFAULT_OUTPUT_LIMIT, Engine, Job, JobState, Missing, Recipe, Request, Stack, Storage,
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

/// One server tick: advance the queue, then save it and the slots back.
pub(crate) fn on_block_entity_tick(
    host: &mut dyn ServerHost,
    event: &mut BlockEntityTickEvent<'_>,
) {
    let loaded = Tag::Compound(event.entity.nbt.clone());
    let mut engine = read_engine(&loaded);
    if engine.is_empty() {
        // Nothing to run: the queue is empty, so the tick changed nothing at all.
        *event.reply = BlockEntityTickReply::Unchanged;
        return;
    }
    let original = event.entity.items.clone();
    let mut storage = storage_of(host, &original);
    let events = engine.tick(&mut storage);
    if events.is_empty() {
        // A waiting job whose ingredients and slots have not changed yet.
        *event.reply = BlockEntityTickReply::Unchanged;
        return;
    }
    event.entity.items = items_of(host, &storage, &original);
    event.entity.nbt = write_engine(&engine);
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
fn read_job(tag: &Tag) -> Option<Job> {
    let cells = match tag.get("cells") {
        Some(Tag::List(cells)) => cells
            .iter()
            .map(|cell| match cell {
                Tag::List(ids) => Some(ids.iter().filter_map(string_of).collect::<Vec<String>>()),
                _ => None,
            })
            .collect::<Option<Vec<Vec<String>>>>()?,
        _ => return None,
    };
    if cells.is_empty() || cells.iter().any(Vec::is_empty) {
        return None;
    }
    let recipe = Recipe::new(
        string(tag, "recipe")?,
        cells,
        string(tag, "out")?,
        count(tag, "out_count")?,
    );
    let blocked = match string(tag, "blocked").as_deref() {
        Some("output-full") => Some(Blocked::OutputFull),
        Some("missing") => Some(Blocked::MissingIngredients(read_missing(tag))),
        _ => None,
    };
    Some(Job {
        id: count(tag, "id")?,
        recipe,
        output_limit: count(tag, "limit")?.max(1),
        requested: count(tag, "requested")?,
        produced: count(tag, "produced")?,
        crafts: count(tag, "crafts")?,
        crafts_needed: count(tag, "crafts_needed")?,
        progress: count(tag, "progress")?,
        state: JobState::from_name(&string(tag, "state")?)?,
        blocked,
    })
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

/// The queue as the block entity saves it.
fn write_engine(engine: &Engine) -> Compound {
    let mut data = Compound::new();
    let jobs = engine.jobs().iter().map(write_job).collect();
    data.insert(JOBS_TAG.into(), Tag::List(jobs));
    data.insert(NEXT_TAG.into(), Tag::Int(engine.next_id() as i32));
    data
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
    data.insert("crafts".into(), Tag::Int(job.crafts as i32));
    data.insert("crafts_needed".into(), Tag::Int(job.crafts_needed as i32));
    data.insert("progress".into(), Tag::Int(job.progress as i32));
    data.insert("state".into(), Tag::String(job.state.name().into()));
    let cells = job
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
    match &job.blocked {
        Some(Blocked::OutputFull) => {
            data.insert("blocked".into(), Tag::String("output-full".into()));
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
    match subcommand {
        "status" => status(host, level, pos),
        "request" => request(host, level, pos, invocation),
        "cancel" => cancel(host, level, pos),
        "feed" => feed(host, level, pos, invocation),
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
    let table = recipes_for(&*host, &output);
    if table.is_empty() {
        reply(
            host,
            &format!("Integrated Crafting: no crafting recipe makes {output}"),
        );
        return;
    }
    let limit = host
        .max_stack_size(item.item())
        .unwrap_or(DEFAULT_OUTPUT_LIMIT);
    let items = slots_of(&*host, entity);
    let storage = storage_of(&*host, &items);
    let mut engine = read_engine(&Tag::Compound(entity.data(&*host).unwrap_or_default()));
    match engine.submit(
        &table,
        &storage,
        &Request::new(output.clone(), wanted),
        limit,
    ) {
        Ok(id) => {
            reply(
                host,
                &format!("Integrated Crafting: job {id} accepted for {wanted} x {output}"),
            );
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

/// `/integratedcrafting feed`: put items into the input slots, as a hopper above the
/// interface does.
fn feed(host: &mut dyn ServerHost, level: LevelRef, pos: [i32; 3], invocation: &CommandInvocation) {
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
    let read: &dyn Host = &*host;
    let items = slots_of(read, entity);
    let model = storage_of(read, &items).inputs;
    let limit = read
        .max_stack_size(item.item())
        .unwrap_or(DEFAULT_OUTPUT_LIMIT);
    let (planned, inserted) = plan_insert(&model, &item.item().id(read), wanted, limit);
    match write_slots(host, entity, &model, &planned) {
        Some(()) => reply(
            host,
            &format!("Integrated Crafting: inserted {inserted} of {wanted}"),
        ),
        None => reply(
            host,
            "Integrated Crafting: the input slots refused the items.",
        ),
    }
}

/// The stacks in an interface's slots, read one by one.
fn slots_of(host: &dyn Host, entity: mod_api::BlockEntityRef) -> Vec<Option<ItemStack>> {
    (0..SLOTS as u32)
        .map(|slot| entity.item_slot(host, slot).ok().flatten())
        .collect()
}

/// The pure recipes for `output`, resolved from the game's crafting table.
///
/// A recipe's cells are its ingredient item ids, deduplicated and sorted so the saved
/// job and its readout do not depend on a set's iteration order. `Ingredient::item_keys`
/// is the exact, component-independent membership; a custom predicate it cannot
/// enumerate falls back to the ingredient's display order, which a job then matches by
/// item id alone.
fn recipes_for(host: &dyn Host, output: &str) -> Vec<Recipe> {
    let mut table = Vec::new();
    for recipe in host.recipes_of_type("minecraft:crafting").iter() {
        let Some(result) = recipe.results.first() else {
            continue;
        };
        if result.item().id(host) != output || recipe.ingredients.is_empty() {
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
            output.to_owned(),
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

/// Write the ingredient slots that changed, in one transaction. `None` when the game
/// refused a write (the transaction is rolled back, so a failure leaves the slots as
/// they were).
fn write_slots(
    host: &mut dyn ServerHost,
    entity: mod_api::BlockEntityRef,
    original: &[Option<Stack>],
    planned: &[Option<Stack>],
) -> Option<()> {
    let items = slots_of(&*host, entity);
    let mut writes = Vec::new();
    for (slot, wanted) in planned.iter().enumerate() {
        if original.get(slot) == Some(wanted) {
            continue;
        }
        writes.push((slot as u32, rebuild(&*host, wanted, items.get(slot))));
    }
    let outcome = host.transaction(|host, tx| {
        for (slot, stack) in &writes {
            entity.set_item_slot(host, tx, *slot, stack.clone())?;
        }
        Ok::<(), mod_api::BlockEntityItemError>(())
    });
    outcome.ok()
}

/// The queue written back into the block entity's saved compound.
fn save(host: &mut dyn ServerHost, entity: mod_api::BlockEntityRef, engine: &Engine) {
    if let Err(error) = entity.replace_data(host, write_engine(engine)) {
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
        let saved = Tag::Compound(write_engine(&engine));
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
        let saved = Tag::Compound(write_engine(&engine));
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

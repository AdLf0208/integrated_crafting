// Pure logic for Integrated Crafting slice 1: the crafting-job engine behind the
// Crafting Interface.
//
// This module has no world access. The support module reads the interface's saved
// job list and its item slots, asks the game for the recipe's ingredients once, at
// submission (upstream resolves a job's recipe when the job starts and keeps it for
// the job's lifetime — see the mod's own info book, "Dynamic Recipes"), and calls
// these rules on every tick. Nothing here knows what a level, a block entity or an
// item stack is: a job is an output id and a count, a recipe is a list of
// ingredient cells, and a slot is an item id and a count.
//
// Upstream (`CraftingJob`, `CraftingJobStatus`, `MissingIngredients`,
// `CraftingHelpers`, `RecipeKey`) has one crafting job hold a recipe, the amount it
// still needs, its status and the ingredients it reserved. This slice keeps the
// observable part of that: a request for N of an item resolves to a recipe and a
// craft count, the job blocks while an ingredient or an output slot is missing, and
// the ingredients are consumed one craft at a time. The dependency graph (a job
// whose ingredient is itself craftable), priorities, multiple crafting CPUs and
// fluids/energy are out of this slice and recorded in `notes.md`.

/// Ticks one craft takes. Upstream measures a machine's recipe duration
/// (`RecipeDurationStatistics`); this slice has no bound machine to measure, so a
/// craft takes the same second a vanilla crafting-table craft is usually given.
pub const TICKS_PER_CRAFT: u32 = 20;

/// The largest stack one output slot holds when the game cannot say
/// (`Host::max_stack_size` answers `None` for an item it does not know).
pub const DEFAULT_OUTPUT_LIMIT: u32 = 64;

/// One item and how many of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stack {
    /// The item id, `minecraft:oak_log`.
    pub item: String,
    /// How many, at least one.
    pub count: u32,
}

impl Stack {
    /// A stack of `count` of `item`.
    pub fn new(item: impl Into<String>, count: u32) -> Self {
        Self {
            item: item.into(),
            count,
        }
    }
}

/// A resolved crafting recipe: one craft takes one item out of each ingredient
/// cell and makes `output_count` of `output`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recipe {
    /// The recipe id, `minecraft:oak_planks`.
    pub id: String,
    /// The ingredient cells, each the item ids that fit it. A shaped recipe
    /// contributes one cell per filled pattern cell, so a recipe needing two planks
    /// has two cells that may name the same item.
    pub ingredients: Vec<Vec<String>>,
    /// The result item id.
    pub output: String,
    /// How many the recipe makes per craft.
    pub output_count: u32,
}

impl Recipe {
    /// A recipe with `ingredients` making `output_count` of `output`.
    pub fn new(
        id: impl Into<String>,
        ingredients: Vec<Vec<String>>,
        output: impl Into<String>,
        output_count: u32,
    ) -> Self {
        Self {
            id: id.into(),
            ingredients,
            output: output.into(),
            output_count,
        }
    }
}

/// What a caller asks the interface to make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    /// The output item id.
    pub output: String,
    /// How many are wanted.
    pub count: u32,
}

impl Request {
    /// A request for `count` of `output`.
    pub fn new(output: impl Into<String>, count: u32) -> Self {
        Self {
            output: output.into(),
            count,
        }
    }
}

/// One ingredient cell a job cannot fill.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    /// The item ids that would have fitted the cell.
    pub candidates: Vec<String>,
    /// How many the job needs from this cell.
    pub needed: u32,
    /// How many the slots still held when the cell was reached.
    pub have: u32,
}

/// Why the engine would not take a request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// No recipe in the supplied table makes the requested item.
    UnknownRecipe,
    /// A request for zero or fewer.
    EmptyRequest,
    /// The recipe exists but this engine cannot consume it: no ingredient cells, or
    /// a result count of zero.
    UnsupportedRecipe,
    /// The slots cannot pay for the whole job. Upstream reserves every ingredient a
    /// job needs when it starts, so a request it cannot pay for is refused rather
    /// than started and starved.
    MissingIngredients(Vec<Missing>),
}

impl Refusal {
    /// The readout line for this refusal.
    pub fn line(&self, output: &str) -> String {
        match self {
            Self::UnknownRecipe => format!("no crafting recipe makes {output}"),
            Self::EmptyRequest => "count must be at least 1".to_owned(),
            Self::UnsupportedRecipe => {
                format!("the recipe of {output} has no ingredients this engine can consume")
            }
            Self::MissingIngredients(missing) => missing_lines(missing),
        }
    }
}

/// The readout text for a list of missing ingredients, `a; b`.
fn missing_lines(missing: &[Missing]) -> String {
    missing
        .iter()
        .map(|one| {
            format!(
                "missing {} x {} (have {})",
                one.needed,
                one.candidates.join("|"),
                one.have
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The interface's slots: `inputs` are the ingredient cells, `outputs` the results.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Storage {
    /// The ingredient slots, in click order. Empty slots are `None`.
    pub inputs: Vec<Option<Stack>>,
    /// The result slots, filled in order.
    pub outputs: Vec<Option<Stack>>,
}

impl Storage {
    /// Slots from their contents.
    pub fn new(inputs: Vec<Option<Stack>>, outputs: Vec<Option<Stack>>) -> Self {
        Self { inputs, outputs }
    }
}

/// Why a running job is not crafting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Blocked {
    /// The slots can no longer pay for the next craft.
    MissingIngredients(Vec<Missing>),
    /// The result slots cannot take the next craft's output.
    OutputFull,
}

impl Blocked {
    /// The readout line for this reason.
    pub fn line(&self) -> String {
        match self {
            Self::MissingIngredients(missing) => missing_lines(missing),
            Self::OutputFull => "output full".to_owned(),
        }
    }
}

/// Where a job stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    /// Waiting for the job before it (upstream's blocking mode).
    Queued,
    /// Crafting.
    Running,
    /// Waiting for ingredients or an output slot.
    Blocked,
}

impl JobState {
    /// The name the saved compound stores.
    pub fn name(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Blocked => "blocked",
        }
    }

    /// The state a saved name stands for; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "queued" => Some(Self::Queued),
            "running" => Some(Self::Running),
            "blocked" => Some(Self::Blocked),
            _ => None,
        }
    }
}

/// One accepted request, with the recipe it was resolved against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    /// The job's id, unique per interface and increasing.
    pub id: u32,
    /// The recipe this job keeps for its whole life, even if the table changes.
    pub recipe: Recipe,
    /// The most of the output one slot holds, read once when the job is accepted.
    pub output_limit: u32,
    /// How many were asked for.
    pub requested: u32,
    /// How many have been made (a whole number of crafts).
    pub produced: u32,
    /// Crafts finished so far.
    pub crafts: u32,
    /// Crafts the request needs: `ceil(requested / output_count)`.
    pub crafts_needed: u32,
    /// Ticks spent on the current craft.
    pub progress: u32,
    /// Its place in the queue.
    pub state: JobState,
    /// Why it stopped crafting, when it did.
    pub blocked: Option<Blocked>,
}

/// What one tick did. An empty list is a tick that changed nothing, which is what
/// the interface answers `Unchanged` for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A queued job began.
    Started { job: u32 },
    /// A tick of the running job's craft.
    Advanced { job: u32, progress: u32 },
    /// One craft finished and its result is in the output slots.
    Crafted {
        job: u32,
        produced: u32,
        requested: u32,
    },
    /// The request is met; the job left the queue.
    Finished { job: u32, produced: u32 },
    /// The running job stopped on a reason.
    Blocked { job: u32 },
    /// A blocked job found its ingredients or its slot again.
    Unblocked { job: u32 },
}

/// The queue behind one interface.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Engine {
    jobs: Vec<Job>,
    next_id: u32,
}

impl Engine {
    /// An empty queue.
    pub fn new() -> Self {
        Self {
            jobs: Vec::new(),
            next_id: 1,
        }
    }

    /// A queue restored from a saved compound.
    pub fn from_parts(jobs: Vec<Job>, next_id: u32) -> Self {
        Self { jobs, next_id }
    }

    /// The jobs, in queue order.
    pub fn jobs(&self) -> &[Job] {
        &self.jobs
    }

    /// The id the next accepted job gets.
    pub fn next_id(&self) -> u32 {
        self.next_id
    }

    /// Whether the queue holds nothing at all.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// The recipe in `table` for `output`, the first one when several make it.
    pub fn recipe_for<'a>(table: &'a [Recipe], output: &str) -> Option<&'a Recipe> {
        table.iter().find(|recipe| recipe.output == output)
    }

    /// Accept a request, resolving it against `table` and checking that `storage`
    /// can pay for the whole job. The new job's id is returned; nothing is consumed
    /// until it crafts.
    ///
    /// `output_limit` is the most of the requested item one output slot holds
    /// (the game's `Item.getDefaultMaxStackSize`, read once here): the engine uses it
    /// to decide whether a craft's result fits, and keeps it with the job.
    pub fn submit(
        &mut self,
        table: &[Recipe],
        storage: &Storage,
        request: &Request,
        output_limit: u32,
    ) -> Result<u32, Refusal> {
        if request.count == 0 {
            return Err(Refusal::EmptyRequest);
        }
        let Some(recipe) = Self::recipe_for(table, &request.output) else {
            return Err(Refusal::UnknownRecipe);
        };
        if recipe.ingredients.is_empty() || recipe.output_count == 0 {
            return Err(Refusal::UnsupportedRecipe);
        }
        let crafts_needed = request.count.div_ceil(recipe.output_count);
        let missing = missing_ingredients(&recipe.ingredients, &storage.inputs, crafts_needed);
        if !missing.is_empty() {
            return Err(Refusal::MissingIngredients(missing));
        }
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.jobs.push(Job {
            id,
            recipe: recipe.clone(),
            output_limit: output_limit.max(1),
            requested: request.count,
            produced: 0,
            crafts: 0,
            crafts_needed,
            progress: 0,
            // The first job starts at once; a later one waits its turn (upstream's
            // blocking mode, which is the default).
            state: if self.jobs.is_empty() {
                JobState::Running
            } else {
                JobState::Queued
            },
            blocked: None,
        });
        Ok(id)
    }

    /// Drop every job the interface has not finished. The number dropped is
    /// returned; the ingredients stay in the slots, where the caller finds them.
    pub fn cancel(&mut self) -> usize {
        let dropped = self.jobs.len();
        self.jobs.clear();
        dropped
    }

    /// Advance the queue by one tick, crafting into `storage`.
    ///
    /// One craft takes [`TICKS_PER_CRAFT`] ticks; the ingredients are consumed and the
    /// result inserted only on the tick a craft completes, so a job that cannot finish
    /// a craft leaves the slots untouched.
    pub fn tick(&mut self, storage: &mut Storage) -> Vec<Event> {
        let Some(index) = self.active() else {
            return self.start_next();
        };
        let job = &mut self.jobs[index];
        let id = job.id;

        if job.state == JobState::Blocked {
            // A blocked job retries every tick: the missing ingredient may arrive in a
            // hopper, or the output slot may empty.
            let Ok(next) = plan_craft(&job.recipe, storage, job.output_limit) else {
                return Vec::new();
            };
            *storage = next;
            job.state = JobState::Running;
            job.blocked = None;
            job.progress = 0;
            job.crafts += 1;
            job.produced += job.recipe.output_count;
            let produced = job.produced;
            let requested = job.requested;
            let mut events = vec![
                Event::Unblocked { job: id },
                Event::Crafted {
                    job: id,
                    produced,
                    requested,
                },
            ];
            if produced >= requested {
                self.jobs.remove(index);
                events.push(Event::Finished { job: id, produced });
                events.extend(self.start_next());
            }
            return events;
        }

        if job.progress < TICKS_PER_CRAFT {
            job.progress += 1;
        }
        if job.progress < TICKS_PER_CRAFT {
            return vec![Event::Advanced {
                job: id,
                progress: job.progress,
            }];
        }
        match plan_craft(&job.recipe, storage, job.output_limit) {
            Err(blocked) => {
                job.state = JobState::Blocked;
                job.blocked = Some(blocked);
                vec![Event::Blocked { job: id }]
            }
            Ok(next) => {
                *storage = next;
                job.progress = 0;
                job.crafts += 1;
                job.produced += job.recipe.output_count;
                let produced = job.produced;
                let requested = job.requested;
                let mut events = vec![Event::Crafted {
                    job: id,
                    produced,
                    requested,
                }];
                if produced >= requested {
                    self.jobs.remove(index);
                    events.push(Event::Finished { job: id, produced });
                    events.extend(self.start_next());
                }
                events
            }
        }
    }

    /// The readout `/integratedcrafting status` prints: one line for the interface,
    /// then one for each job that is not waiting its turn.
    pub fn readout(&self, at: [i32; 3]) -> Vec<String> {
        let [x, y, z] = at;
        let active = self.active().map(|index| &self.jobs[index]);
        let summary = match active {
            Some(job) if job.state == JobState::Blocked => format!("blocked job {}", job.id),
            Some(job) => format!("running job {}", job.id),
            None => "idle".to_owned(),
        };
        let queued = self
            .jobs
            .iter()
            .filter(|job| job.state == JobState::Queued)
            .count();
        let head = if queued == 0 {
            format!("Integrated Crafting: interface at {x},{y},{z}: {summary}")
        } else {
            format!("Integrated Crafting: interface at {x},{y},{z}: {summary} ({queued} queued)")
        };
        let mut lines = vec![head];
        for job in &self.jobs {
            let id = job.id;
            if let Some(blocked) = &job.blocked {
                lines.push(format!(
                    "Integrated Crafting: job {id}: blocked: {}",
                    blocked.line()
                ));
                continue;
            }
            if job.state == JobState::Queued {
                lines.push(format!("Integrated Crafting: job {id}: queued"));
                continue;
            }
            lines.push(format!(
                "Integrated Crafting: job {id}: {} x{} (recipe {}), craft {}/{}, progress {}/{TICKS_PER_CRAFT}, produced {}",
                job.recipe.output,
                job.requested,
                job.recipe.id,
                job.crafts,
                job.crafts_needed,
                job.progress,
                job.produced
            ));
        }
        lines
    }

    /// The job the interface is working on, if any.
    fn active(&self) -> Option<usize> {
        self.jobs
            .iter()
            .position(|job| matches!(job.state, JobState::Running | JobState::Blocked))
    }

    /// Begin the first job waiting its turn.
    fn start_next(&mut self) -> Vec<Event> {
        let Some(index) = self
            .jobs
            .iter()
            .position(|job| job.state == JobState::Queued)
        else {
            return Vec::new();
        };
        self.jobs[index].state = JobState::Running;
        vec![Event::Started {
            job: self.jobs[index].id,
        }]
    }
}

/// How many items each ingredient cell still needs, by cell, for `crafts` crafts.
///
/// Cells that accept the same items are counted together, so a recipe with two cells
/// of one item asks for two per craft: the count a player would see as the job's
/// total need for that item. The `have` of a missing ingredient is what the slots can
/// still give it after the ingredients before it took theirs.
fn missing_ingredients(
    ingredients: &[Vec<String>],
    inputs: &[Option<Stack>],
    crafts: u32,
) -> Vec<Missing> {
    // The whole job's need per distinct candidate set, in first-seen order.
    let mut groups: Vec<(Vec<String>, u32)> = Vec::new();
    for cell in ingredients {
        match groups.iter_mut().find(|(known, _)| known == cell) {
            Some((_, needed)) => *needed += crafts,
            None => groups.push((cell.clone(), crafts)),
        }
    }
    let mut left: Vec<u32> = inputs
        .iter()
        .map(|slot| slot.as_ref().map_or(0, |stack| stack.count))
        .collect();
    let mut missing = Vec::new();
    for (candidates, needed) in groups {
        let mut taken = 0;
        for slot in inputs.iter().zip(left.iter_mut()) {
            let Some(stack) = slot.0 else {
                continue;
            };
            if !candidates.contains(&stack.item) {
                continue;
            }
            let take = (*slot.1).min(needed - taken);
            *slot.1 -= take;
            taken += take;
            if taken == needed {
                break;
            }
        }
        if taken < needed {
            missing.push(Missing {
                candidates,
                needed,
                have: taken,
            });
        }
    }
    missing
}

/// One craft's result on copies of the slots, or why it cannot happen.
///
/// Both the ingredients and the output are planned before anything is applied, so a
/// craft that cannot finish consumes nothing.
fn plan_craft(recipe: &Recipe, storage: &Storage, output_limit: u32) -> Result<Storage, Blocked> {
    let mut left = storage.inputs.to_vec();
    let mut taken: Vec<(Vec<String>, u32)> = Vec::new();
    for cell in &recipe.ingredients {
        let Some(index) = left.iter().position(|slot| {
            slot.as_ref()
                .is_some_and(|stack| stack.count > 0 && cell.contains(&stack.item))
        }) else {
            match taken.iter_mut().find(|(candidates, _)| candidates == cell) {
                Some((_, count)) => *count += 1,
                None => taken.push((cell.clone(), 1)),
            }
            continue;
        };
        let slot = left[index]
            .as_mut()
            .expect("the slot was matched as a stack above");
        slot.count -= 1;
        if slot.count == 0 {
            left[index] = None;
        }
    }
    if !taken.is_empty() {
        let missing = taken
            .into_iter()
            .map(|(candidates, needed)| Missing {
                candidates,
                needed,
                have: 0,
            })
            .collect();
        return Err(Blocked::MissingIngredients(missing));
    }

    let mut filled = storage.outputs.to_vec();
    let mut remaining = recipe.output_count;
    for slot in &mut filled {
        let Some(stack) = slot else {
            continue;
        };
        if stack.item != recipe.output || stack.count >= output_limit {
            continue;
        }
        let room = output_limit - stack.count;
        let add = room.min(remaining);
        stack.count += add;
        remaining -= add;
        if remaining == 0 {
            break;
        }
    }
    while remaining > 0 {
        let Some(empty) = filled.iter().position(Option::is_none) else {
            return Err(Blocked::OutputFull);
        };
        let add = output_limit.min(remaining);
        filled[empty] = Some(Stack::new(recipe.output.clone(), add));
        remaining -= add;
    }
    Ok(Storage::new(left, filled))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-log-to-four-planks recipe, the tutorial's example.
    fn planks() -> Recipe {
        Recipe::new(
            "minecraft:oak_planks",
            vec![vec!["minecraft:oak_log".to_owned()]],
            "minecraft:oak_planks",
            4,
        )
    }

    /// Two sticks per two planks, the vanilla recipe.
    fn sticks() -> Recipe {
        Recipe::new(
            "minecraft:stick",
            vec![
                vec!["minecraft:oak_planks".to_owned()],
                vec!["minecraft:oak_planks".to_owned()],
            ],
            "minecraft:stick",
            4,
        )
    }

    fn slots(inputs: &[(&str, u32)], outputs: &[(&str, u32)]) -> Storage {
        Storage::new(
            inputs
                .iter()
                .map(|(item, count)| Some(Stack::new(*item, *count)))
                .collect(),
            outputs
                .iter()
                .map(|(item, count)| Some(Stack::new(*item, *count)))
                .collect(),
        )
    }

    fn good_outputs() -> Vec<Option<Stack>> {
        vec![None, None]
    }

    #[test]
    fn a_submitted_job_completes_and_consumes_its_ingredients() {
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            vec![Some(Stack::new("minecraft:oak_log", 2))],
            good_outputs(),
        );
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("two logs pay for eight planks");

        let mut events: Vec<Event> = Vec::new();
        for _ in 0..(TICKS_PER_CRAFT * 2) {
            events.extend(engine.tick(&mut storage));
        }
        assert_eq!(
            events.last(),
            Some(&Event::Finished {
                job: id,
                produced: 8
            })
        );
        assert_eq!(storage.inputs, vec![None]);
        assert_eq!(
            storage.outputs,
            vec![Some(Stack::new("minecraft:oak_planks", 8)), None]
        );
        assert!(engine.is_empty(), "a finished job leaves the queue");
    }

    #[test]
    fn a_job_with_a_missing_ingredient_is_refused_at_submission() {
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_planks", 1)], &[]);
        let refusal = engine
            .submit(
                &[sticks()],
                &storage,
                &Request::new("minecraft:stick", 4),
                64,
            )
            .expect_err("one plank cannot pay for two cells");
        assert_eq!(
            refusal,
            Refusal::MissingIngredients(vec![Missing {
                candidates: vec!["minecraft:oak_planks".to_owned()],
                needed: 2,
                have: 1,
            }])
        );
        assert!(engine.is_empty(), "a refused request queues nothing");
    }

    #[test]
    fn a_job_larger_than_the_supplied_ingredients_is_refused_with_the_deficit() {
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_log", 3)], &[]);
        let refusal = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 100),
                64,
            )
            .expect_err("25 logs are needed for 100 planks");
        assert_eq!(
            refusal,
            Refusal::MissingIngredients(vec![Missing {
                candidates: vec!["minecraft:oak_log".to_owned()],
                needed: 25,
                have: 3,
            }])
        );
    }

    #[test]
    fn a_craft_shows_partial_progress_before_it_completes() {
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            vec![Some(Stack::new("minecraft:oak_log", 1))],
            good_outputs(),
        );
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("one log pays for four planks");
        assert_eq!(
            engine.tick(&mut storage),
            vec![Event::Advanced {
                job: id,
                progress: 1
            }],
            "a submitted job starts at once, so its first tick is a tick of its craft"
        );
        for tick in 2..TICKS_PER_CRAFT {
            assert_eq!(
                engine.tick(&mut storage),
                vec![Event::Advanced {
                    job: id,
                    progress: tick
                }]
            );
            assert_eq!(
                storage.inputs,
                vec![Some(Stack::new("minecraft:oak_log", 1))],
                "nothing is consumed before the craft completes"
            );
        }
        assert_eq!(
            engine.tick(&mut storage),
            vec![
                Event::Crafted {
                    job: id,
                    produced: 4,
                    requested: 4
                },
                Event::Finished {
                    job: id,
                    produced: 4
                }
            ]
        );
        assert_eq!(storage.inputs, vec![None]);
        assert_eq!(
            storage.outputs,
            vec![Some(Stack::new("minecraft:oak_planks", 4)), None]
        );
    }

    #[test]
    fn a_cancelled_job_never_consumes_anything() {
        let mut engine = Engine::new();
        let mut storage = slots(&[("minecraft:oak_log", 2)], &[]);
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("two logs pay for eight planks");
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("a second job queues behind the first");
        for _ in 0..(TICKS_PER_CRAFT - 1) {
            engine.tick(&mut storage);
        }
        assert_eq!(engine.jobs().first().map(|job| job.id), Some(id));
        assert_eq!(engine.cancel(), 2);
        assert!(engine.is_empty());
        assert_eq!(
            storage.inputs,
            vec![Some(Stack::new("minecraft:oak_log", 2))],
            "a cancelled job leaves its ingredients in the slots"
        );
        assert!(engine.tick(&mut storage).is_empty());
    }

    #[test]
    fn a_job_whose_output_does_not_fit_is_blocked_and_consumes_nothing() {
        let mut engine = Engine::new();
        // The output slots hold another item in every slot.
        let mut storage = slots(
            &[("minecraft:oak_log", 1)],
            &[("minecraft:stick", 64), ("minecraft:stick", 64)],
        );
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("the ingredients are there; the output is the problem");
        for _ in 0..TICKS_PER_CRAFT {
            engine.tick(&mut storage);
        }
        assert_eq!(engine.jobs()[0].blocked, Some(Blocked::OutputFull));
        assert_eq!(engine.jobs()[0].state, JobState::Blocked);
        assert_eq!(
            storage.inputs,
            vec![Some(Stack::new("minecraft:oak_log", 1))],
            "a craft that does not fit consumes nothing"
        );
        assert_eq!(id, 1);
        assert_eq!(
            engine.tick(&mut storage),
            Vec::new(),
            "a blocked job retries quietly while nothing has changed"
        );
    }

    #[test]
    fn a_blocked_job_resumes_when_the_output_slot_is_freed() {
        let mut engine = Engine::new();
        let mut storage = slots(
            &[("minecraft:oak_log", 1)],
            &[("minecraft:stick", 64), ("minecraft:stick", 64)],
        );
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("the ingredients are there");
        for _ in 0..(TICKS_PER_CRAFT + 1) {
            engine.tick(&mut storage);
        }
        assert_eq!(engine.jobs()[0].blocked, Some(Blocked::OutputFull));

        storage.outputs[0] = None;
        assert_eq!(
            engine.tick(&mut storage),
            vec![
                Event::Unblocked { job: id },
                Event::Crafted {
                    job: id,
                    produced: 4,
                    requested: 4
                },
                Event::Finished {
                    job: id,
                    produced: 4
                }
            ]
        );
        assert_eq!(
            storage.outputs,
            vec![
                Some(Stack::new("minecraft:oak_planks", 4)),
                Some(Stack::new("minecraft:stick", 64))
            ],
            "the freed slot takes the result; the other keeps its own item"
        );
    }

    #[test]
    fn a_blocked_job_whose_ingredient_is_taken_blocks_with_the_deficit() {
        let mut engine = Engine::new();
        let mut storage = slots(&[("minecraft:oak_log", 1)], &[]);
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("one log pays for one craft");
        storage.inputs[0] = None;
        for _ in 0..(TICKS_PER_CRAFT - 1) {
            engine.tick(&mut storage);
        }
        assert_eq!(engine.tick(&mut storage), vec![Event::Blocked { job: id }]);
        assert_eq!(
            engine.jobs()[0].blocked,
            Some(Blocked::MissingIngredients(vec![Missing {
                candidates: vec!["minecraft:oak_log".to_owned()],
                needed: 1,
                have: 0,
            }]))
        );
    }

    #[test]
    fn a_recipe_that_makes_more_than_requested_stops_at_the_first_surplus_craft() {
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            vec![Some(Stack::new("minecraft:oak_log", 2))],
            good_outputs(),
        );
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 5),
                64,
            )
            .expect("two logs pay for two crafts");
        assert_eq!(engine.jobs()[0].crafts_needed, 2);
        for _ in 0..(TICKS_PER_CRAFT * 2) {
            engine.tick(&mut storage);
        }
        assert_eq!(
            storage.outputs[0],
            Some(Stack::new("minecraft:oak_planks", 8)),
            "a craft makes the recipe's whole output"
        );
        assert!(engine.is_empty());
        assert_eq!(id, 1);
    }

    #[test]
    fn two_cells_that_accept_the_same_item_are_accounted_separately() {
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_planks", 1)], &[]);
        let refusal = engine
            .submit(
                &[sticks()],
                &storage,
                &Request::new("minecraft:stick", 4),
                64,
            )
            .expect_err("one plank cannot fill two cells");
        assert_eq!(
            refusal,
            Refusal::MissingIngredients(vec![Missing {
                candidates: vec!["minecraft:oak_planks".to_owned()],
                needed: 2,
                have: 1,
            }])
        );
    }

    #[test]
    fn an_unknown_output_is_refused() {
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_log", 1)], &[]);
        assert_eq!(
            engine.submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:diamond", 1),
                64
            ),
            Err(Refusal::UnknownRecipe)
        );
        assert_eq!(
            engine.submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 0),
                64
            ),
            Err(Refusal::EmptyRequest)
        );
    }

    #[test]
    fn a_recipe_without_ingredient_cells_is_refused() {
        let mut engine = Engine::new();
        let storage = slots(&[], &[]);
        let special = Recipe::new("mymod:free", vec![], "mymod:free", 1);
        assert_eq!(
            engine.submit(&[special], &storage, &Request::new("mymod:free", 1), 64),
            Err(Refusal::UnsupportedRecipe)
        );
    }

    #[test]
    fn the_readout_names_the_queue_the_running_job_and_its_progress() {
        let mut engine = Engine::new();
        let mut storage = slots(&[("minecraft:oak_log", 4)], &[]);
        assert_eq!(
            engine.readout([10, 64, -3]),
            vec!["Integrated Crafting: interface at 10,64,-3: idle".to_owned()]
        );
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("four logs pay for eight planks");
        assert_eq!(
            engine.readout([10, 64, -3]),
            vec![
                "Integrated Crafting: interface at 10,64,-3: running job 1".to_owned(),
                "Integrated Crafting: job 1: minecraft:oak_planks x8 (recipe minecraft:oak_planks), craft 0/2, progress 0/20, produced 0".to_owned(),
            ]
        );
        engine.tick(&mut storage);
        for _ in 0..7 {
            engine.tick(&mut storage);
        }
        assert_eq!(
            engine.readout([10, 64, -3]),
            vec![
                "Integrated Crafting: interface at 10,64,-3: running job 1".to_owned(),
                "Integrated Crafting: job 1: minecraft:oak_planks x8 (recipe minecraft:oak_planks), craft 0/2, progress 8/20, produced 0".to_owned(),
            ]
        );
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("the second job queues");
        assert_eq!(
            engine.readout([10, 64, -3])[0],
            "Integrated Crafting: interface at 10,64,-3: running job 1 (1 queued)"
        );
        assert_eq!(
            engine.readout([10, 64, -3])[2],
            "Integrated Crafting: job 2: queued"
        );
    }

    #[test]
    fn the_readout_names_a_blocked_reason() {
        let mut engine = Engine::new();
        let mut storage = slots(
            &[("minecraft:oak_log", 1)],
            &[("minecraft:stick", 64), ("minecraft:stick", 64)],
        );
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("the ingredients are there");
        for _ in 0..(TICKS_PER_CRAFT + 1) {
            engine.tick(&mut storage);
        }
        assert_eq!(
            engine.readout([0, 1, 2]),
            vec![
                "Integrated Crafting: interface at 0,1,2: blocked job 1".to_owned(),
                "Integrated Crafting: job 1: blocked: output full".to_owned(),
            ]
        );
        let refusal = Refusal::MissingIngredients(vec![Missing {
            candidates: vec!["minecraft:oak_log".to_owned()],
            needed: 2,
            have: 1,
        }]);
        assert_eq!(
            refusal.line("minecraft:oak_planks"),
            "missing 2 x minecraft:oak_log (have 1)"
        );
    }
}

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

/// The most jobs one interface's queue holds. Upstream's queue is a UI-driven list a player adds
/// to one job at a time; with no cap, a player can grow the saved compound and the per-tick
/// rebuild of it without limit (review 1270-integratedcrafting, "a queue without a cap").
pub const MAX_QUEUE: usize = 8;

/// How many dependency steps one job may plan. Upstream's graph is unbounded but detects cycles;
/// this engine plans the whole chain at submission and stores it in the saved compound, so a
/// depth keeps a pathological recipe table from growing that compound without limit. A chain
/// deeper than this is refused with its deficit, never followed.
pub const MAX_DEPENDENCY_DEPTH: usize = 8;

/// How far from the interface a player may be and still start, cancel or feed a job: vanilla's
/// 4.5-block block-interaction range plus the server's own one-block buffer
/// (`ServerPlayer.canInteractWithBlock`'s `+ 1.0`). The console, a command block and a function
/// are the server's own hand and act at any distance.
pub const REACH_BLOCKS: f64 = 6.0;

/// Whether a player at `from` is within `reach` blocks of the block at `to`, measured to the
/// block's centre as the server's interaction check does.
pub fn within_reach(from: [f64; 3], to: [i32; 3], reach: f64) -> bool {
    let centre = [
        f64::from(to[0]) + 0.5,
        f64::from(to[1]) + 0.5,
        f64::from(to[2]) + 0.5,
    ];
    let dx = from[0] - centre[0];
    let dy = from[1] - centre[1];
    let dz = from[2] - centre[2];
    dx * dx + dy * dy + dz * dz <= reach * reach
}

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
    /// The queue already holds [`MAX_QUEUE`] jobs.
    QueueFull,
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
            Self::QueueFull => format!("the queue is full ({MAX_QUEUE} jobs)"),
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
    /// The ingredient slots cannot take a dependency step's output.
    InputFull,
}

impl Blocked {
    /// The readout line for this reason.
    pub fn line(&self) -> String {
        match self {
            Self::MissingIngredients(missing) => missing_lines(missing),
            Self::OutputFull => "output full".to_owned(),
            Self::InputFull => "ingredient slots full".to_owned(),
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

/// Where a step's product goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sink {
    /// Into the ingredient slots: a later step of the same job spends it (a dependency's output).
    Inputs,
    /// Into the result slots: the job's product, waiting for a hopper.
    Results,
}

impl Sink {
    /// The name the saved compound stores.
    pub fn name(self) -> &'static str {
        match self {
            Self::Inputs => "inputs",
            Self::Results => "results",
        }
    }

    /// The sink a saved name stands for; `None` for anything else.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "inputs" => Some(Self::Inputs),
            "results" => Some(Self::Results),
            _ => None,
        }
    }
}

/// One recipe a job runs, and how far it has got. A job with one step is a plain request; a chain
/// has the dependencies first and the requested recipe last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// The recipe this step crafts.
    pub recipe: Recipe,
    /// Crafts this step needs.
    pub crafts_needed: u32,
    /// Crafts finished so far.
    pub crafts: u32,
    /// Ticks spent on the current craft.
    pub progress: u32,
    /// Where the step's product goes.
    pub sink: Sink,
    /// The most of the step's output one slot holds.
    pub limit: u32,
}

/// One accepted request, with the recipe it was resolved against and the chain that makes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    /// The job's id, unique per interface and increasing.
    pub id: u32,
    /// The recipe the request was resolved to (the last step of the chain).
    pub recipe: Recipe,
    /// The most of the requested output one slot holds, read once when the job is accepted.
    pub output_limit: u32,
    /// How many were asked for.
    pub requested: u32,
    /// How many have been made (a whole number of crafts of the last step).
    pub produced: u32,
    /// The chain, in the order it runs: dependencies first, the requested recipe last.
    pub steps: Vec<Step>,
    /// Its place in the queue.
    pub state: JobState,
    /// Why it stopped crafting, when it did.
    pub blocked: Option<Blocked>,
}

impl Job {
    /// The step being crafted, or `None` when every step has finished.
    pub fn current_step(&self) -> Option<&Step> {
        self.steps
            .iter()
            .find(|step| step.crafts < step.crafts_needed)
    }

    /// The index of [`Self::current_step`] in [`Self::steps`].
    pub fn current_index(&self) -> Option<usize> {
        self.steps
            .iter()
            .position(|step| step.crafts < step.crafts_needed)
    }

    /// Whether every step has finished.
    pub fn is_finished(&self) -> bool {
        self.current_index().is_none()
    }
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
    /// One craft of a dependency step finished; its product is in the ingredient slots.
    DependencyCrafted { job: u32, step: usize },
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

    /// Accept a request, resolving it against `table` and checking that `storage` can pay for the
    /// whole chain it needs. The new job's id is returned; nothing is consumed until it crafts.
    ///
    /// A missing ingredient that is itself craftable becomes a dependency step, planned first and
    /// stored with the job: that is upstream's job dependency graph, bounded by
    /// [`MAX_DEPENDENCY_DEPTH`] and broken at a repeated output id. What the chain has to take
    /// from the slots is reserved at submission, so a second job cannot spend it.
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
        if self.jobs.len() >= MAX_QUEUE {
            return Err(Refusal::QueueFull);
        }
        let Some(recipe) = Self::recipe_for(table, &request.output) else {
            return Err(Refusal::UnknownRecipe);
        };
        if recipe.ingredients.is_empty() || recipe.output_count == 0 {
            return Err(Refusal::UnsupportedRecipe);
        }
        let limit = output_limit.max(1);
        let mut free = self.free_inputs(&storage.inputs);
        let mut path = vec![request.output.clone()];
        let steps = match plan_recipe(table, &mut free, recipe, request.count, &mut path, 0, limit)
        {
            Ok(steps) => steps,
            Err(missing) => return Err(Refusal::MissingIngredients(missing)),
        };
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.jobs.push(Job {
            id,
            recipe: recipe.clone(),
            output_limit: limit,
            requested: request.count,
            produced: 0,
            steps,
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

    /// The ingredient slots as a new job would find them once every queued job has run: the
    /// simulation walks each job's remaining steps in order, takes what they consume and adds what
    /// their dependency steps produce, so a chain's intermediate output is not counted twice.
    fn free_inputs(&self, inputs: &[Option<Stack>]) -> Vec<Option<Stack>> {
        let mut free = inputs.to_vec();
        for job in &self.jobs {
            for step in &job.steps {
                let left = step.crafts_needed.saturating_sub(step.crafts);
                if left == 0 {
                    continue;
                }
                for (candidates, cells) in cell_groups(&step.recipe.ingredients) {
                    let _ = reserve(&mut free, &candidates, cells.saturating_mul(left));
                }
                if step.sink == Sink::Inputs {
                    add_output(
                        &mut free,
                        &step.recipe.output,
                        left.saturating_mul(step.recipe.output_count),
                        step.limit,
                    );
                }
            }
        }
        free
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
    /// a craft leaves the slots untouched. A step's product goes to the result slots for
    /// the requested recipe, and to the ingredient slots for a dependency.
    pub fn tick(&mut self, storage: &mut Storage) -> Vec<Event> {
        let Some(index) = self.active() else {
            return self.start_next();
        };
        if self.jobs[index].state == JobState::Blocked {
            // A blocked job retries every tick: the missing ingredient may arrive in a
            // hopper, or the output slot may empty.
            let Some(step_index) = self.jobs[index].current_index() else {
                return Vec::new();
            };
            let step = &self.jobs[index].steps[step_index];
            let (sink, limit) = (step.sink, step.limit);
            let recipe = step.recipe.clone();
            let Ok(next) = plan_craft(&recipe, storage, sink, limit) else {
                return Vec::new();
            };
            *storage = next;
            self.jobs[index].state = JobState::Running;
            self.jobs[index].blocked = None;
            let id = self.jobs[index].id;
            let mut events = vec![Event::Unblocked { job: id }];
            events.extend(self.finish_craft(index, step_index));
            return events;
        }
        let Some(step_index) = self.jobs[index].current_index() else {
            return Vec::new();
        };
        let step = &self.jobs[index].steps[step_index];
        let (progress, sink, limit) = (step.progress, step.sink, step.limit);
        let recipe = step.recipe.clone();
        if progress + 1 < TICKS_PER_CRAFT {
            let job = &mut self.jobs[index];
            job.steps[step_index].progress = progress + 1;
            return vec![Event::Advanced {
                job: job.id,
                progress: progress + 1,
            }];
        }
        match plan_craft(&recipe, storage, sink, limit) {
            Err(blocked) => {
                let job = &mut self.jobs[index];
                job.state = JobState::Blocked;
                job.blocked = Some(blocked);
                vec![Event::Blocked { job: job.id }]
            }
            Ok(next) => {
                *storage = next;
                self.finish_craft(index, step_index)
            }
        }
    }

    /// Record that the craft of `steps[step_index]` completed and its product is in `storage`:
    /// count it, emit the event, and finish the job (or move it to its next step).
    fn finish_craft(&mut self, index: usize, step_index: usize) -> Vec<Event> {
        let (id, produced, finished, event) = {
            let job = &mut self.jobs[index];
            let step = &mut job.steps[step_index];
            step.crafts += 1;
            step.progress = 0;
            let event = if step.sink == Sink::Results {
                job.produced = job.produced.saturating_add(step.recipe.output_count);
                Event::Crafted {
                    job: job.id,
                    produced: job.produced,
                    requested: job.requested,
                }
            } else {
                Event::DependencyCrafted {
                    job: job.id,
                    step: step_index,
                }
            };
            (job.id, job.produced, job.is_finished(), event)
        };
        let mut events = vec![event];
        if finished {
            self.jobs.remove(index);
            events.push(Event::Finished { job: id, produced });
            events.extend(self.start_next());
        }
        events
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
            let Some(index) = job.current_index() else {
                continue;
            };
            let step = &job.steps[index];
            // A chain names the step it is on; a plain request keeps the line it always had.
            let step_tag = if job.steps.len() > 1 {
                format!(", step {}/{}", index + 1, job.steps.len())
            } else {
                String::new()
            };
            lines.push(format!(
                "Integrated Crafting: job {id}: {} x{} (recipe {}{step_tag}), craft {}/{}, progress {}/{TICKS_PER_CRAFT}, produced {}",
                job.recipe.output,
                job.requested,
                step.recipe.id,
                step.crafts,
                step.crafts_needed,
                step.progress,
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

/// The ingredient cells grouped by the items that fit them: a shaped recipe with two cells of the
/// same item asks for two per craft, so the group carries how many cells it stands for.
fn cell_groups(ingredients: &[Vec<String>]) -> Vec<(Vec<String>, u32)> {
    let mut groups: Vec<(Vec<String>, u32)> = Vec::new();
    for cell in ingredients {
        match groups.iter_mut().find(|(known, _)| known == cell) {
            Some((_, cells)) => *cells += 1,
            None => groups.push((cell.clone(), 1)),
        }
    }
    groups
}

/// Take up to `need` items that fit `candidates` out of `free`, in slot order, and return how
/// many were taken. Slots that cannot pay keep what they have (a job already blocked does not
/// reserve what is not there).
fn reserve(free: &mut [Option<Stack>], candidates: &[String], need: u32) -> u32 {
    let mut taken = 0;
    for slot in free.iter_mut() {
        let Some(stack) = slot else {
            continue;
        };
        if !candidates.contains(&stack.item) {
            continue;
        }
        let take = stack.count.min(need - taken);
        stack.count -= take;
        taken += take;
        if taken == need {
            break;
        }
    }
    taken
}

/// Add `count` of `item` to the free slots, merging into stacks of the same item first and then
/// filling empty slots. What does not fit is dropped: the engine's own craft would block, so no
/// later job can spend it either way.
fn add_output(free: &mut [Option<Stack>], item: &str, count: u32, limit: u32) {
    let mut remaining = count;
    for slot in free.iter_mut() {
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
            return;
        }
    }
    while remaining > 0 {
        let Some(empty) = free.iter().position(Option::is_none) else {
            return;
        };
        let add = limit.min(remaining);
        free[empty] = Some(Stack::new(item.to_owned(), add));
        remaining -= add;
    }
}

/// Plan the steps that make `count` of `recipe`'s output, dependencies first.
///
/// `free` is what the slots can still give; every leaf item the plan takes is removed from it, so
/// two sibling dependencies cannot spend the same item. A missing cell whose item is craftable
/// becomes a dependency, tried candidate by candidate (the cell's item ids are sorted by the
/// caller) until one plans within `free`; the first failure's cell is what a refused request
/// reports. `path` holds the outputs on the current chain, so a recipe that needs itself is not
/// followed. `depth` is 0 for the requested recipe and the number of dependencies below it.
fn plan_recipe(
    table: &[Recipe],
    free: &mut Vec<Option<Stack>>,
    recipe: &Recipe,
    count: u32,
    path: &mut Vec<String>,
    depth: usize,
    top_limit: u32,
) -> Result<Vec<Step>, Vec<Missing>> {
    let crafts = count.div_ceil(recipe.output_count);
    if crafts == 0 {
        return Ok(Vec::new());
    }
    let mut steps = Vec::new();
    let mut missing = Vec::new();
    for (candidates, cells) in cell_groups(&recipe.ingredients) {
        let need = cells.saturating_mul(crafts);
        let taken = reserve(free, &candidates, need);
        let shortfall = need - taken;
        if shortfall == 0 {
            continue;
        }
        let mut planned = false;
        for candidate in &candidates {
            if path.contains(candidate) || depth + 1 > MAX_DEPENDENCY_DEPTH {
                continue;
            }
            let Some(child) = Engine::recipe_for(table, candidate) else {
                continue;
            };
            if child.ingredients.is_empty() || child.output_count == 0 {
                continue;
            }
            let mut trial = free.clone();
            path.push(candidate.clone());
            let attempt = plan_recipe(
                table,
                &mut trial,
                child,
                shortfall,
                path,
                depth + 1,
                top_limit,
            );
            path.pop();
            if let Ok(child_steps) = attempt {
                *free = trial;
                steps.extend(child_steps);
                planned = true;
                break;
            }
        }
        if !planned {
            missing.push(Missing {
                candidates,
                needed: need,
                have: taken,
            });
        }
    }
    if !missing.is_empty() {
        return Err(missing);
    }
    steps.push(Step {
        recipe: recipe.clone(),
        crafts_needed: crafts,
        crafts: 0,
        progress: 0,
        // The requested recipe's product waits in the result slots; a dependency's goes back to
        // the ingredient slots for the step above it.
        sink: if depth == 0 {
            Sink::Results
        } else {
            Sink::Inputs
        },
        limit: if depth == 0 {
            top_limit
        } else {
            DEFAULT_OUTPUT_LIMIT
        },
    });
    Ok(steps)
}

/// Fold an external change to the interface's container into its saved slots.
///
/// The interface keeps its 13 slots in its saved compound (the one place both a command and the
/// tick can read and write) and mirrors them into the block entity's item container, which is what
/// hoppers reach. `mirror` is what it last wrote there: a slot whose container value differs from
/// the mirror was changed from outside — a hopper put something in, or took something out — and
/// that value wins; every other slot keeps the saved value, so a container the game has not
/// refreshed never overwrites a job's ingredients. A saved slot past the container's length keeps
/// its value, and a container slot past the saved length is taken.
pub fn merge_container(
    saved: &[Option<Stack>],
    mirror: &[Option<Stack>],
    container: &[Option<Stack>],
) -> Vec<Option<Stack>> {
    let len = saved.len().max(container.len());
    (0..len)
        .map(|slot| {
            let held = container.get(slot).cloned().flatten();
            let shown = mirror.get(slot).cloned().flatten();
            if held != shown {
                held
            } else {
                saved.get(slot).cloned().flatten()
            }
        })
        .collect()
}

/// One craft's result on copies of the slots, or why it cannot happen.
///
/// Both the ingredients and the output are planned before anything is applied, so a
/// craft that cannot finish consumes nothing. A dependency's product goes to the
/// ingredient slots (where the step above it spends it); the requested recipe's goes to
/// the result slots.
fn plan_craft(
    recipe: &Recipe,
    storage: &Storage,
    sink: Sink,
    output_limit: u32,
) -> Result<Storage, Blocked> {
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

    let mut filled = match sink {
        Sink::Inputs => left.clone(),
        Sink::Results => storage.outputs.to_vec(),
    };
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
            return Err(match sink {
                Sink::Inputs => Blocked::InputFull,
                Sink::Results => Blocked::OutputFull,
            });
        };
        let add = output_limit.min(remaining);
        filled[empty] = Some(Stack::new(recipe.output.clone(), add));
        remaining -= add;
    }
    Ok(match sink {
        Sink::Inputs => Storage::new(filled, storage.outputs.clone()),
        Sink::Results => Storage::new(left, filled),
    })
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

    /// The interface's nine ingredient slots, as the block entity declares them: a chain needs
    /// room for a dependency's product beside the items the next step spends.
    fn grid(entries: &[Option<(&str, u32)>]) -> Vec<Option<Stack>> {
        let mut slots: Vec<Option<Stack>> = entries
            .iter()
            .map(|entry| entry.map(|(item, count)| Stack::new(item, count)))
            .collect();
        slots.resize(9, None);
        slots
    }

    #[test]
    fn the_queue_has_a_cap() {
        // Eight logs, eight jobs of one log each: the ninth is refused, so a player cannot grow
        // the saved compound (and the per-tick rebuild of it) without limit.
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_log", 64)], &[]);
        for _ in 0..MAX_QUEUE {
            engine
                .submit(
                    &[planks()],
                    &storage,
                    &Request::new("minecraft:oak_planks", 4),
                    64,
                )
                .expect("the queue has room");
        }
        assert_eq!(engine.jobs().len(), MAX_QUEUE);
        assert_eq!(
            engine
                .submit(
                    &[planks()],
                    &storage,
                    &Request::new("minecraft:oak_planks", 4),
                    64
                )
                .expect_err("the queue is full"),
            Refusal::QueueFull
        );
        assert_eq!(
            engine.jobs().len(),
            MAX_QUEUE,
            "a refused request queues nothing"
        );
        assert_eq!(MAX_QUEUE, 8, "the cap the readout and the refusal name");
    }

    #[test]
    fn a_queued_job_reserves_the_ingredients_it_still_needs() {
        // Two logs in the slots. The first job needs both for eight planks, so the second job
        // cannot claim the same logs again: it is refused with the deficit that is actually left.
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_log", 2)], &[]);
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("two logs pay for eight planks");
        let refusal = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect_err("the first job reserved both logs");
        assert_eq!(
            refusal,
            Refusal::MissingIngredients(vec![Missing {
                candidates: vec!["minecraft:oak_log".to_owned()],
                needed: 1,
                have: 0,
            }]),
            "the refusal counts what the queue has already reserved"
        );
        assert_eq!(engine.jobs().len(), 1);
    }

    #[test]
    fn a_reservation_is_released_as_the_job_crafts() {
        // One job for eight planks over four logs: after its first craft, the log it consumed is
        // free for a second job, so the reservation tracks the remaining crafts, not the whole job.
        let mut engine = Engine::new();
        let mut storage = slots(&[("minecraft:oak_log", 4)], &[]);
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("four logs are more than the two the job needs");
        for _ in 0..TICKS_PER_CRAFT {
            engine.tick(&mut storage);
        }
        engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect("the first job has one craft left and the slots still hold three logs");
    }

    #[test]
    fn an_external_change_to_the_container_wins_over_the_saved_slots() {
        // The interface keeps its slots in its saved data and mirrors them into the container the
        // hoppers reach. A slot that no longer holds what the mirror says was changed from
        // outside (a hopper put something in, or took something out): it wins. Every other slot
        // keeps the saved value, so a stale container never overwrites the job's ingredients.
        let saved = vec![
            Some(Stack::new("minecraft:oak_log", 2)),
            None,
            Some(Stack::new("minecraft:oak_planks", 4)),
        ];
        let mirror = vec![
            Some(Stack::new("minecraft:oak_log", 2)),
            None,
            Some(Stack::new("minecraft:oak_planks", 4)),
        ];
        let container = vec![
            Some(Stack::new("minecraft:oak_log", 1)), // a hopper took one
            Some(Stack::new("minecraft:stick", 3)),   // a hopper put some in
            Some(Stack::new("minecraft:oak_planks", 4)),
        ];
        assert_eq!(
            merge_container(&saved, &mirror, &container),
            vec![
                Some(Stack::new("minecraft:oak_log", 1)),
                Some(Stack::new("minecraft:stick", 3)),
                Some(Stack::new("minecraft:oak_planks", 4)),
            ]
        );
    }

    #[test]
    fn a_first_tick_takes_the_container_as_it_stands() {
        // Nothing mirrored yet (a block entity saved before this build): the container is the
        // truth, so an interface that already holds items keeps them.
        let container = vec![Some(Stack::new("minecraft:oak_log", 3))];
        assert_eq!(
            merge_container(&[], &[], &container),
            vec![Some(Stack::new("minecraft:oak_log", 3))]
        );
        // A container shorter than the storage keeps the saved slots past its length.
        let saved = vec![None, Some(Stack::new("minecraft:stone", 1))];
        assert_eq!(merge_container(&saved, &[], &[]), saved);
        // But a slot the mirror showed and the container no longer holds was emptied outside.
        assert_eq!(merge_container(&saved, &saved, &[]), vec![None, None]);
    }

    #[test]
    fn a_player_must_be_within_reach_of_the_interface() {
        let at = [300, 101, 0];
        assert!(
            within_reach([300.5, 101.0, 0.5], at, REACH_BLOCKS),
            "standing in it"
        );
        assert!(
            within_reach([300.5, 101.0, 5.0], at, REACH_BLOCKS),
            "five blocks away"
        );
        assert!(
            !within_reach([300.5, 101.0, 40.0], at, REACH_BLOCKS),
            "across the arena"
        );
        assert!(
            !within_reach([300.5, 140.0, 0.5], at, REACH_BLOCKS),
            "far above it"
        );
        assert_eq!(
            REACH_BLOCKS, 6.0,
            "vanilla's 4.5-block reach plus its own 1-block buffer"
        );
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
        // Four logs: two for the first job's two crafts and one for the second's, with one spare.
        // Two would no longer do: a queued job reserves what it still needs, so the second job
        // cannot claim the logs the first one has reserved (that is its own test).
        let mut storage = slots(&[("minecraft:oak_log", 4)], &[]);
        let id = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 8),
                64,
            )
            .expect("four logs pay for eight planks");
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
            vec![Some(Stack::new("minecraft:oak_log", 4))],
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
        assert_eq!(
            engine.jobs()[0]
                .current_step()
                .expect("the craft is still running")
                .crafts_needed,
            2
        );
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

    /// A recipe making one handle from two sticks: the third link of a log -> planks -> sticks
    /// -> handle chain.
    fn handles() -> Recipe {
        Recipe::new(
            "mymod:handle",
            vec![
                vec!["minecraft:stick".to_owned()],
                vec!["minecraft:stick".to_owned()],
            ],
            "mymod:handle",
            1,
        )
    }

    #[test]
    fn a_request_crafts_a_craftable_intermediate_before_the_requested_recipe() {
        // Upstream's job dependency graph: the stick recipe needs planks, the slots hold none, but
        // the planks are craftable from the log the job was given. The dependency runs first, its
        // output lands in the ingredient slots, and the requested recipe then spends it.
        let mut engine = Engine::new();
        let mut storage = Storage::new(grid(&[Some(("minecraft:oak_log", 1))]), good_outputs());
        engine
            .submit(
                &[planks(), sticks()],
                &storage,
                &Request::new("minecraft:stick", 4),
                64,
            )
            .expect("one log makes four planks, and two of those make four sticks");
        for _ in 0..(TICKS_PER_CRAFT - 1) {
            engine.tick(&mut storage);
        }
        assert_eq!(
            storage.inputs.first(),
            Some(&Some(Stack::new("minecraft:oak_log", 1))),
            "the log is not consumed before the planks craft completes"
        );
        engine.tick(&mut storage);
        assert_eq!(
            storage.inputs.first(),
            Some(&Some(Stack::new("minecraft:oak_planks", 4))),
            "the dependency's output lands in the ingredient slots"
        );
        for _ in 0..TICKS_PER_CRAFT {
            engine.tick(&mut storage);
        }
        assert_eq!(
            storage.outputs.first(),
            Some(&Some(Stack::new("minecraft:stick", 4))),
            "the requested recipe's output is the job's product"
        );
        assert_eq!(
            storage.inputs.first(),
            Some(&Some(Stack::new("minecraft:oak_planks", 2))),
            "two planks fill the stick recipe's two cells; the other two are left over"
        );
        assert!(engine.is_empty(), "the whole chain finished");
    }

    #[test]
    fn a_chain_three_deep_runs_from_the_leaf_up_and_reports_its_steps() {
        let mut engine = Engine::new();
        let mut storage = Storage::new(grid(&[Some(("minecraft:oak_log", 1))]), good_outputs());
        engine
            .submit(
                &[planks(), sticks(), handles()],
                &storage,
                &Request::new("mymod:handle", 1),
                64,
            )
            .expect("one log pays for the planks, the sticks and the handle");
        let readout = engine.readout([0, 1, 2]);
        assert!(
            readout.get(1).is_some_and(|line| line.contains("step 1/3")),
            "the readout names the step a chain is on: {readout:?}"
        );
        assert!(
            readout
                .get(1)
                .is_some_and(|line| line.contains("recipe minecraft:oak_planks")),
            "the first step is the leaf-most dependency: {readout:?}"
        );
        for _ in 0..(TICKS_PER_CRAFT * 3) {
            engine.tick(&mut storage);
        }
        assert_eq!(
            storage.outputs.first(),
            Some(&Some(Stack::new("mymod:handle", 1)))
        );
        // 4 planks - 2 for the sticks = 2 planks, and 4 sticks - 2 for the handle = 2 sticks.
        assert_eq!(
            &storage.inputs[..2],
            &[
                Some(Stack::new("minecraft:oak_planks", 2)),
                Some(Stack::new("minecraft:stick", 2))
            ]
        );
        assert!(
            storage.inputs[2..].iter().all(Option::is_none),
            "the other ingredient slots stay empty"
        );
        assert!(engine.is_empty());
    }

    #[test]
    fn a_recipe_cycle_is_refused_instead_of_followed() {
        let a = Recipe::new("mymod:a", vec![vec!["mymod:b".to_owned()]], "mymod:a", 1);
        let b = Recipe::new("mymod:b", vec![vec!["mymod:a".to_owned()]], "mymod:b", 1);
        let mut engine = Engine::new();
        let storage = slots(&[], &[]);
        let refusal = engine
            .submit(&[a, b], &storage, &Request::new("mymod:a", 1), 64)
            .expect_err("neither recipe can start without the other");
        assert_eq!(
            refusal,
            Refusal::MissingIngredients(vec![Missing {
                candidates: vec!["mymod:b".to_owned()],
                needed: 1,
                have: 0,
            }]),
            "the refusal names the missing cell, not a cycle"
        );
        assert!(engine.is_empty());
    }

    #[test]
    fn a_dependency_chain_deeper_than_the_cap_is_refused_even_with_its_leaf_supplied() {
        // A chain of ten links, with the deepest item in the slots: a graph without a depth cap
        // would craft it; the cap keeps the saved plan bounded.
        let depth = 10usize;
        let mut table = Vec::new();
        for link in 0..depth {
            table.push(Recipe::new(
                format!("mymod:link{}", link + 1),
                vec![vec![format!("mymod:link{link}")]],
                format!("mymod:link{}", link + 1),
                1,
            ));
        }
        let mut engine = Engine::new();
        let storage = slots(&[("mymod:link0", 1)], &[]);
        let refusal = engine
            .submit(
                &table,
                &storage,
                &Request::new(format!("mymod:link{depth}"), 1),
                64,
            )
            .expect_err("the chain is deeper than the cap");
        assert!(
            matches!(refusal, Refusal::MissingIngredients(_)),
            "a too-deep chain is a deficit, not a panic: {refusal:?}"
        );
        assert_eq!(MAX_DEPENDENCY_DEPTH, 8, "the cap the record names");
    }

    #[test]
    fn ingredients_in_the_slots_are_spent_before_a_dependency_is_planned() {
        // Two planks and one log, eight sticks wanted: the planks in the slots cover one stick
        // craft, the log makes the second, so the planks step crafts one recipe, not two.
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            grid(&[
                Some(("minecraft:oak_planks", 2)),
                Some(("minecraft:oak_log", 1)),
            ]),
            good_outputs(),
        );
        engine
            .submit(
                &[planks(), sticks()],
                &storage,
                &Request::new("minecraft:stick", 8),
                64,
            )
            .expect("the slots and the log together pay for eight sticks");
        for _ in 0..(TICKS_PER_CRAFT * 3) {
            engine.tick(&mut storage);
        }
        assert_eq!(
            storage.outputs.first(),
            Some(&Some(Stack::new("minecraft:stick", 8)))
        );
        assert_eq!(
            storage.inputs.first(),
            Some(&Some(Stack::new("minecraft:oak_planks", 2))),
            "the log's four planks are spent two at a time; the slots' two planks went first"
        );
        assert!(engine.is_empty());
    }

    #[test]
    fn a_dependency_whose_output_cannot_fit_blocks_and_consumes_nothing() {
        // The intermediate recipe makes 70 of its output: one free slot holds 64, and the other
        // ingredient slot is filled with another item, so the step cannot store its product. The
        // log must not be consumed.
        let bulk = Recipe::new(
            "mymod:bulk",
            vec![vec!["minecraft:oak_log".to_owned()]],
            "mymod:bulk",
            70,
        );
        let thing = Recipe::new(
            "mymod:thing",
            vec![vec!["mymod:bulk".to_owned()]],
            "mymod:thing",
            1,
        );
        let mut engine = Engine::new();
        let mut storage = Storage::new(
            grid(&[
                Some(("minecraft:oak_log", 1)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
                Some(("minecraft:stone", 64)),
            ]),
            good_outputs(),
        );
        engine
            .submit(
                &[bulk, thing],
                &storage,
                &Request::new("mymod:thing", 1),
                64,
            )
            .expect("the log pays for the intermediate");
        for _ in 0..(TICKS_PER_CRAFT + 1) {
            engine.tick(&mut storage);
        }
        assert_eq!(
            engine.jobs()[0].blocked,
            Some(Blocked::InputFull),
            "the dependency's output cannot be stored"
        );
        assert!(
            engine.readout([0, 0, 0])[1].contains("blocked: ingredient slots full"),
            "the readout names the full ingredient slots"
        );
        assert_eq!(
            storage.inputs.first(),
            Some(&Some(Stack::new("minecraft:oak_log", 1))),
            "a craft that cannot store its output consumes nothing"
        );
        assert_eq!(storage.outputs, good_outputs(), "nothing was produced");
    }

    #[test]
    fn a_queued_chain_reserves_the_leaf_ingredients_it_still_needs() {
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_log", 1)], &[]);
        engine
            .submit(
                &[planks(), sticks()],
                &storage,
                &Request::new("minecraft:stick", 4),
                64,
            )
            .expect("the log is reserved for the chain");
        let refusal = engine
            .submit(
                &[planks()],
                &storage,
                &Request::new("minecraft:oak_planks", 4),
                64,
            )
            .expect_err("the chain has already reserved the only log");
        assert_eq!(
            refusal,
            Refusal::MissingIngredients(vec![Missing {
                candidates: vec!["minecraft:oak_log".to_owned()],
                needed: 1,
                have: 0,
            }])
        );
    }

    #[test]
    fn a_craftable_intermediate_that_cannot_be_paid_for_is_refused_with_the_cell_deficit() {
        // One plank and no log: the missing plank is craftable, but its own ingredient is absent,
        // so the deficit is the cell the job wanted (the planks tag), not the leaf.
        let mut engine = Engine::new();
        let storage = slots(&[("minecraft:oak_planks", 1)], &[]);
        let refusal = engine
            .submit(
                &[planks(), sticks()],
                &storage,
                &Request::new("minecraft:stick", 4),
                64,
            )
            .expect_err("one plank cannot fill two cells and no log can be turned into more");
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

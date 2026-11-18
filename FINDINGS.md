# Integrated Crafting 1.8.1 — what the artefact says

Provenance: `integratedcrafting-1.21.1-neoforge-1.8.1.jar` from the ATM10 pack,
kept at `ports/_shared/licence-cache/` (SHA-256 in `LICENSE-NOTICE.md`). Read as
the jar ships it: the manifest, the data pack, the assets, the info book's text
and the public API classes. No decompilation.

## Manifest

`META-INF/neoforge.mods.toml`: `modId="integratedcrafting"`, `version="1.8.1"`,
`license="MIT"`, authors "rubensworks (aka kroeser)", `displayName="IntegratedCrafting"`,
description "Craft stuff in Integrated Dynamics networks." Required dependencies:
`cyclopscore [1.30.0,)`, `integrateddynamics [1.37.1,)`, `commoncapabilities [2.11.5,)`.
So it is an **add-on** of Integrated Dynamics, and its network is Integrated
Dynamics' (`ICraftingNetwork`, registered through `Capabilities.CraftingNetwork`).

## Content (300 entries, no blocks)

- **4 blockstates**: `part_interface_crafting`, `..._attuned`, `..._enumerated`,
  `part_crafting_writer`. These are the multipart models the Integrated Dynamics
  cable renderer draws for the four **parts**; there is no `data/integratedcrafting/
  tags/block`, no loot table and no block declaration in the jar. The parts are
  **items** placed on cable faces, exactly like `integrateddynamics:part_redstone_reader`.
- 4 item models, 10 block models (the interface's own geometry is
  `models/block/interface_crafting.json`, 10 cubes, parented to
  `integrateddynamics:block/part`), 15 part/aspect textures, 1 GUI texture set.
- **6 recipes**, all `minecraft:crafting_shaped`, for the four part items plus
  `part_interface_crafting_attuned_recipes` and its offsets. The interface's
  recipe needs `integrateddynamics:variable_transformer_output`,
  `..._input` and `integrateddynamics:crystalized_menril_block`.
- 7 advancements, 2 info-book XMLs (the mod's in-game manual), 1 structure NBT
  (a 10×10 test platform for its gametests), 29 lang files.

## Behaviour, from the info book and the API classes

- The **Crafting Interface** is placed on a cable and *points at a machine*; it
  exposes the recipes it holds (variable cards) to the network. The attuned and
  enumerated variants expose a machine's whole recipe list instead.
- **A job pulls its ingredients from the network** and holds them, "to avoid
  ingredients being used elsewhere before the crafting jobs are completed", then
  inserts them into the bound machine. The interface does **not** import the
  machine's results; a job completes only when the expected outputs reach the
  network's storage. The crafting table is the exception: the interface crafts
  with it itself.
- **Blocking mode is on by default**: one recipe at a time, the next only after
  the previous completed. Distribution over several interfaces requires each to
  expose the same recipe, and reusable ingredients and dynamic recipes are
  part-specific features.
- **Dynamic recipes**: a running job keeps the recipe it started with; only new
  jobs use the changed recipe. That is why this port resolves a recipe once, at
  submission, and stores it with the job.
- API surface: `api/crafting/CraftingJob` (amount, status, `CraftingJobDependencyGraph`),
  `CraftingJobStatus`, `MissingIngredients` (an element list with `PrototypedWithRequested`),
  `RecipeKey`, `ICraftingNetwork` (`getCraftingJobs`, `getRecipes`, `getCraftingIngredients`),
  `CraftingJobFinishedEvent`, and `CraftingHelpers` (the resolver).
- Commands: `/integratedcrafting craft …` (the gametest's crafting-job generator).

## What this port takes, and what it leaves

Taken: the **job** as an object with an amount, a recipe resolved once and kept,
ingredients held, one craft at a time, a blocked state with the missing
ingredients named, completion when the requested amount is made, cancellation,
and a readout of the queue.

Left (recorded in `notes.md`): the network (no cross-mod API at this pin), the
variable-card layer and the part screens, the attuned/enumerated variants, the
bound machine and its item capability, priorities, distributed jobs, the
dependency graph (a job whose ingredient is itself craftable), reusable
ingredients, fluids and energy, the aspects and the terminals integration.

# Licence notice

This port is a clean-room Rust rewrite of **Integrated Crafting 1.8.1** by
rubensworks (CyclopsMC), distributed under the **MIT License**.

- Upstream: <https://github.com/CyclopsMC/IntegratedCrafting>. The pack jar is
  release `1.8.1` for Minecraft 1.21.1 NeoForge; this slice was written from the
  jar's own data, its info book (the mod's documentation) and its published API
  classes, not from decompiled code.
- Licence read from the pack jar's `META-INF/neoforge.mods.toml`
  (`license="MIT"`, `modId="integratedcrafting"`); the MIT text is the one the
  Cyclops projects ship (Copyright (c) 2015 Cyclops), copied verbatim into
  `LICENSE`.
- Pack jar: `integratedcrafting-1.21.1-neoforge-1.8.1.jar`, SHA-256
  `44b2ac7b36260933f1e131da098cb2a0e472a99d1d04185b7f3aa94ba555e3ff`, kept at
  `ports/_shared/licence-cache/integratedcrafting-1.21.1-neoforge-1.8.1.jar`.
- Carried content (the interface's texture, its model geometry, its recipe
  shape and its name) is the upstream project's MIT-licensed data. The block
  model's parent, the blockstate's single state, the block declaration, the
  loot table and the recipe's key substitutions are authored here: upstream's
  interface is a part drawn on a cable, not a block, and its recipe names parts
  of Integrated Dynamics this campaign has not ported.

The port is **local-only** until the maintainer explicitly approves publishing
(campaign ARCHITECTURE.md §8). A published copy must keep the MIT notice above.

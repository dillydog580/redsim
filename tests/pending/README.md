# Pending cases

Builds where redsim knowingly differs from the real game, because the difference comes from
something redsim doesn't simulate yet. redsim reports a warning for each one, so the difference is
never silent. Each file holds the game's recorded output, ready to become a regular regression test
once the missing behaviour is supported.

- Items dropped from broken containers (item entities), falling into hoppers or onto pressure plates.
- A hopper pulling items out of a composter (composters as containers).
- Primed TNT (an entity) falling onto pressure plates or tripwire.
- A jukebox's disc popping out as an item entity onto tripwire when the jukebox is replaced.
- A sculk sensor powering TNT whose primed entity bounces on slime: the bounce is a vibration that
  another sensor hears.

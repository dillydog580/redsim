# redsim for AI agents

You can design, test and debug Minecraft 26.3 redstone entirely in code. Results are exactly what
the game does (verified tick by tick), and deterministic: rerunning reproduces a failure exactly.

## The loop

```python
import redsim as rs

w = rs.World(trace=True)          # trace=True makes explain() work; costs some speed
# 1. build (exact states, no updates)        or: w = rs.load("x.litematic", origin=..., trace=True)
w.fill((0, 0, 0), (6, 0, 2), "stone")
...
w.reshape()                       # fix dust/fence/wall/stair connections without redstone updates

# 2. run with inputs (callables run between ticks, before the given tick)
tl = w.record(60, inputs={0: lambda w: w.use((0, 1, 0)), 30: lambda w: w.use((0, 1, 0))})

# 3. check
assert tl.first((3, 2, 0), "air") is not None
print(tl.history((3, 2, 0)))      # [(tick, Block), ...] only when it changed
print(tl.flickers())              # changes undone within the same tick (0-tick pulses)
print(w.warnings)                 # non-empty = something not simulated mattered: result not exact

# 4. understand a failure
print(w.explain((3, 2, 0)))       # cause chain of its last change, back to your input
print(w.why_not_piston((3, 1, 0)))  # power sources incl. quasi-connectivity; why it can't push
print(w.pending())                # scheduled ticks / block events still queued
print(w.render(y=1))              # text picture of a layer
```

`w.copy()` clones a world; worlds tick in parallel threads (the GIL is released), so try design
variants concurrently. `w.run_until_idle(max_ticks)` returns how long the build took to settle
(`== max_ticks` means it never did: a clock or an oscillation).

More tools for the loop:

* `w.record(ticks, inputs, probes=[pos, ...])`: `tl.probes[pos]` is the power at pos on every tick,
  including ticks where no block changed (useful for pulse lengths).
* `w.nets()` / `w.net_of(pos)`: dust networks, the blocks each one powers and the sources touching
  it. Two signals that should be separate but share a net, or a net that powers a block it
  shouldn't, show up here before you run anything.
* `w.route(source, target)`: lays dust from a source to a target that touches nothing else, checks
  it with `nets()` and a differential simulation, and only then writes it (`RouteError` otherwise).
* `w.render(view="front", at=z)` / `view="side"`: vertical slices, for stairs, towers and doors.
* `w.verify(inputs, ticks)`: run the same world in the real game and compare every tick.

## Making a hand-built world consistent

Raw placement (`set`, `fill`, `paste`) updates nothing, so a pasted circuit can sit in an
impossible state (dust at power 0 next to a lit torch). Recipe:

1. Build the circuit with its inputs in the state you want (levers on or off).
2. `w.settle()`: every block gets one neighbour update (y, z, x order) and the world runs until
   idle. Pistons and observers are skipped by default so they don't fire during setup.
3. Place pistons and observers in the state that matches (`settle(skip=())` lets them react too).
4. Now record your test.

## Gotchas (true in the real game, so in redsim)

* **Repeater/comparator `facing`** is the side the *input* comes from; the output is the opposite
  side. `repeater[facing=west]` carries a signal west -> east. Observer `facing` is the side it
  watches; it outputs from its back. The helpers avoid this: `w.repeater(p, toward="east")`,
  `w.comparator(p, toward=...)`, `w.observer(p, watches=...)`, `w.torch(p, attached="north")`,
  `w.lever(p, attached="down")`, `w.door(p, facing=...)` (both halves).
* **Dust shape matters.** An isolated dust placed by a player is a *cross* (powers all four sides);
  a *dot* (all sides `none`) only powers the block below it. `wire()`/`reshape()` give the shapes
  the game computes for the neighbours present.
* **Quasi-connectivity:** pistons, droppers and dispensers also count power that would activate a
  block one above them, but only notice it when they get a block update (BUD behaviour).
* **Location dependence:** dust update order and the order pistons clear moved blocks follow Java
  hash order of block positions, so some builds behave differently at different coordinates. Test
  at the real coordinates, or run `redsim locational` / compare runs at a few origins.
* **Inputs are between ticks.** A lever flip at tick 0 powers dust immediately (shown as tick 0
  changes); repeaters/torches/observers react in later ticks. 1 redstone tick = 2 game ticks.
* **Timing trap: a player input reaches pistons one tick later than a circuit signal.** A lever
  flipped between ticks powers a piston right away, but the piston's block event is queued and runs
  in the next tick's block-event phase. A repeater turning on during tick t powers the piston
  during tick t's scheduled ticks, so its block event runs later in the same tick t. A module tested
  with a lever can therefore be one tick off from how it behaves inside a circuit. Drive test
  inputs through a repeater (lever -> repeater -> module) to get circuit timing.
* **A torch beside a comparator does not count as a side input.** A comparator's side reads dust,
  redstone blocks, and sources that strongly power that side (repeaters, comparators, observers
  pointing into it). A torch strongly powers only the block above it.
* **Dust beside a comparator's side does count, at its full power, whatever direction the dust
  points.** It doesn't have to point into the comparator.
* **In subtract mode, the output is the rear input minus the side**, so it can never exceed the
  rear input: weak rear wiring means weak output.
* **A comparator re-reads its inputs when its scheduled tick fires**, so a 2-tick pulse can vanish
  entirely. Use observers (or repeaters) for 2-tick pulses.
* **Dust stairs built on glass carry signal upward only.** Dust steps down only onto a conductor.
* **A block powered by dust still powers pistons beside it, and pistons below it through
  quasi-connectivity.** Check `w.nets()` (blocks each net powers) and `w.why_not_piston(pos)`.
* **Raw placement doesn't update anything.** A pasted build sits as given until something changes
  next to it, like a schematic paste in the game. Use `setblock`/`place` for edits that should
  cause updates.
* **Daylight detectors** read settled sky light and the day clock: set `w.day_time` (0 sunrise,
  6000 noon, 18000 midnight) and `w.advance_time` (default True, like a new world; False holds the
  time). They update every 20 game ticks. `w.sky_light(pos)` shows what a detector would see.
* **Sculk sensors** hear game events from every simulated block within 8 blocks (calibrated: 16):
  pistons, levers, buttons, doors, note blocks, block placing/breaking... The event is selected the
  next tick and travels floor(distance) ticks; output power falls with distance; wool blocks block
  and wool dampens. `w.block_entity(pos)` shows the sensor's last frequency and the
  vibration in flight (`sculk:<freq>:<event>:<ticks left>`). Only block events exist in redsim: no footsteps or other entity vibrations.
* **Not simulated:** entities (players or mobs on pressure plates and tripwire, item drops,
  minecarts on detector rails, TNT explosions, falling sand), fluids, dispenser item behaviours,
  crafting, weather, random ticks. Hoppers and droppers moving items between containers *are*
  simulated exactly. Any of these that would have mattered appear in `w.warnings`.

## Test specs (no Python needed)

```json
{"build": "door.litematic", "ticks": 60,
 "inputs": [{"at": 0, "action": "use", "pos": [0, 1, 0]}],
 "expect": [{"by": 5, "pos": [3, 2, 0], "is": "air"},
            {"from": 5, "to": 25, "pos": [3, 1, 0], "is": "sticky_piston[extended=true]"},
            {"settles": true}]}
```

`redsim test spec.json [--json]`: exit code 0 when everything holds; failures include the block's
history and the cause chain of its last change.

## Checking against the real game

`redsim verify build.litematic --input 0:use:2,1,0 --ticks 60` runs the build in the actual game
(headless) and in redsim and reports the first divergence, if any. Needs `python tools/setup.py`
once.

"""Wiring assistant (redsim.route): offline tests, no real-game oracle needed."""

import pytest

import redsim as rs
from redsim.route import RouteError, route


def ground(w, a=(-4, 0, -6), b=(36, 0, 6)):
    w.fill(a, b, "stone")
    return w


def snapshot(w):
    return sorted(w._w.blocks())


def heights(path):
    """Height changes between consecutive route cells."""
    return [b[1] - a[1] for a, b in zip(path, path[1:])]


def separate(w, result):
    """The route's dust forms networks that contain no other dust."""
    mine = {p for p, _ in result.dust}
    for net in w.nets():
        ds = set(net.dust)
        if ds & mine:
            assert ds <= mine, f"route network includes foreign dust {sorted(ds - mine)}"


def test_straight_route():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((8, 1, 0), "redstone_lamp")
    before = snapshot(w)
    r = route(w, (0, 1, 0), (8, 1, 0))
    assert r.path == [(x, 1, 0) for x in range(1, 8)]
    assert r.repeaters == [] and r.supports == [] and r.delay_ticks == 0
    assert [p for p, _ in r.dust] == r.path
    assert r.end_power == 15 - 6
    assert all(w[p] == "redstone_wire" for p in r.path)
    assert {p for p, _ in r.placed} == set(r.path)
    assert "simulation" in r.report and "nets" in r.report
    separate(w, r)
    w.use((0, 1, 0))
    w.tick(1)
    assert w[(8, 1, 0)] == "redstone_lamp[lit=true]"
    assert w.power((7, 1, 0)) == r.end_power
    # undo restores the world exactly
    w.use((0, 1, 0))
    w.tick(10)
    r.undo(w)
    assert snapshot(w) == before


def test_route_around_obstacle():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((10, 1, 0), "redstone_lamp")
    w.fill((5, 1, -2), (5, 3, 2), "obsidian")            # a wall across the straight line
    r = route(w, (0, 1, 0), (10, 1, 0))
    assert all(p[0] != 5 or abs(p[2]) > 2 for p in r.path)
    assert len(r.path) > 9                               # longer than the straight line
    separate(w, r)
    w.use((0, 1, 0))
    w.tick(2)
    assert w[(10, 1, 0)] == "redstone_lamp[lit=true]"


def test_long_route_needs_repeater():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((30, 1, 0), "redstone_lamp")
    r = route(w, (0, 1, 0), (30, 1, 0))
    assert len(r.repeaters) == 1
    (rp, toward), = r.repeaters
    assert toward == "east"
    assert w[rp] == "repeater[facing=west,delay=1]"      # input from the west, output east
    assert r.delay_ticks == 2
    assert all(power >= 1 for _, power in r.dust)
    assert len(r.nets) == 2                              # the repeater splits the line in two networks
    separate(w, r)
    tl = w.record(10, inputs={0: lambda w: w.use((0, 1, 0))})
    assert tl.first((30, 1, 0), "redstone_lamp[lit=true]") == r.delay_ticks


def test_stays_separate_from_existing_powered_line():
    w = ground(rs.World())
    # an existing, powered dust line one block beside the straight path, lighting its own lamp
    w.set((1, 1, 1), "redstone_block")
    w.wire((2, 1, 1), (7, 1, 1))
    w.set((8, 1, 1), "redstone_lamp")
    w.settle()
    assert w[(8, 1, 1)] == "redstone_lamp[lit=true]"
    old = w.nets()
    assert len(old) == 1
    old_net = old[0]

    w.lever((0, 1, 0))
    w.set((10, 1, 0), "redstone_lamp")
    r = route(w, (0, 1, 0), (10, 1, 0))
    mine = set(r.path)
    # never within connection reach of the existing dust
    for p in mine:
        for q in old_net.dust:
            assert not (abs(p[0] - q[0]) + abs(p[2] - q[2]) == 1 and abs(p[1] - q[1]) <= 1), (p, q)
    # the existing network is unchanged and still separate
    nets = w.nets()
    same = [n for n in nets if n.dust == old_net.dust]
    assert len(same) == 1
    assert same[0].powers == old_net.powers and same[0].sources == old_net.sources
    assert all(not (set(n.dust) & mine) or set(n.dust) <= mine for n in nets)
    # behaviour: the route toggles only its own lamp
    old_states = {p: w[p] for p in old_net.dust}
    w.use((0, 1, 0))
    w.tick(4)
    assert w[(10, 1, 0)] == "redstone_lamp[lit=true]"
    assert w[(8, 1, 1)] == "redstone_lamp[lit=true]"
    assert {p: w[p] for p in old_net.dust} == old_states
    w.use((0, 1, 0))
    w.tick(10)
    assert w[(10, 1, 0)] == "redstone_lamp[lit=false]"
    assert w[(8, 1, 1)] == "redstone_lamp[lit=true]"
    assert {p: w[p] for p in old_net.dust} == old_states


def test_one_step_up_and_one_step_down():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((8, 1, 0), "redstone_lamp")
    w.fill((4, 1, -6), (4, 1, 6), "stone")               # a low wall across the whole box: climb it
    r = route(w, (0, 1, 0), (8, 1, 0), box=((-2, 0, -3), (10, 4, 3)))
    dys = heights(r.path)
    assert dys.count(1) == 1 and dys.count(-1) == 1
    assert (4, 2, 0) in r.path                           # on top of the wall
    assert r.supports == []                              # the wall itself carries the dust
    separate(w, r)
    w.use((0, 1, 0))
    w.tick(1)
    assert w[(8, 1, 0)] == "redstone_lamp[lit=true]"
    assert w.power((4, 2, 0)) > 0


def test_no_path_leaves_world_unchanged():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((10, 1, 0), "redstone_lamp")
    w.fill((5, 1, -6), (5, 4, 6), "obsidian")            # too high to climb inside the box, no way around
    before = snapshot(w)
    with pytest.raises(RouteError) as e:
        route(w, (0, 1, 0), (10, 1, 0), box=((-2, 0, -6), (12, 3, 6)))
    assert "no route" in str(e.value)
    assert e.value.reasons                               # why cells were rejected
    assert snapshot(w) == before


def test_does_not_power_piston_through_the_floor():
    """A piston set into the floor next to the straight line would be powered through the floor block
    the dust stands on (and is quasi-powered from above): the route must keep away from it."""
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((8, 1, 0), "redstone_lamp")
    w.piston((4, 0, 1), "up")
    r = route(w, (0, 1, 0), (8, 1, 0))
    assert (4, 1, 0) not in r.path and (4, 1, 1) not in r.path
    w.use((0, 1, 0))
    w.tick(4)
    assert w[(8, 1, 0)] == "redstone_lamp[lit=true]"
    assert w[(4, 0, 1)] == "piston[extended=false]"


def test_repeater_source_and_piston_target():
    w = ground(rs.World())
    w.lever((-2, 1, 0))
    w.wire((-1, 1, 0))
    w.repeater((0, 1, 0), toward="east")
    w.piston((6, 1, 0), "north")
    w.reshape()
    r = route(w, (0, 1, 0), (6, 1, 0))
    assert r.path[0] == (1, 1, 0)                        # the repeater's output side
    assert (6, 1, -1) not in r.path and (6, 2, 0) not in r.path   # not in front of / on the piston
    w.use((-2, 1, 0))
    w.tick(6)
    assert w[(6, 1, 0)] == "piston[extended=true]"


def test_target_next_to_another_lamp_needs_allow():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.set((6, 1, 0), "redstone_lamp")
    w.set((7, 1, 0), "redstone_lamp")                    # lamps conduct: powering one lights the other
    before = snapshot(w)
    with pytest.raises(RouteError):
        route(w, (0, 1, 0), (6, 1, 0))
    assert snapshot(w) == before
    r = route(w, (0, 1, 0), (6, 1, 0), allow=[(7, 1, 0)])
    w.use((0, 1, 0))
    w.tick(1)
    assert w[(6, 1, 0)] == "redstone_lamp[lit=true]" and w[(7, 1, 0)] == "redstone_lamp[lit=true]"
    assert r.target == (6, 1, 0)


def test_end_at_air_cell_and_update_when_source_on():
    w = ground(rs.World())
    w.lever((0, 1, 0))
    w.use((0, 1, 0))                                     # the source is already on
    w.tick(1)
    r = route(w, (0, 1, 0), (0, 1, 5))
    assert r.target is None and r.path[-1] == (0, 1, 5)
    assert w.power((0, 1, 5)) == r.end_power > 0         # update=True: powered right away


def test_bad_arguments():
    w = ground(rs.World())
    w.set((3, 1, 0), "redstone_lamp")
    with pytest.raises(ValueError):
        route(w, (0, 1, 0), (3, 1, 0))                   # start is air
    w.wire((0, 1, 0))
    with pytest.raises(ValueError):
        route(w, (0, 1, 0), (3, 1, 0))                   # start is dust

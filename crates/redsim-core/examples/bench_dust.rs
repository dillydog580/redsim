use redsim_core::data::data;
use redsim_core::pos::BlockPos;
use redsim_core::World;

fn s(t: &str) -> u32 {
    data().parse_state(t).unwrap()
}

/// A 32x32 plane of dust on stone, toggled by a lever every 20 ticks.
fn main() {
    let mut w = World::new();
    let n = 32;
    for x in 0..n {
        for z in 0..n {
            w.place_raw(BlockPos::new(x, 0, z), s("stone"));
            w.place_raw(BlockPos::new(x, 1, z), s("redstone_wire"));
        }
    }
    for x in 0..n {
        for z in 0..n {
            w.reshape(BlockPos::new(x, 1, z));
        }
    }
    w.place_raw(BlockPos::new(-1, 0, 0), s("stone"));
    w.place_raw(BlockPos::new(-1, 1, 0), s("lever[face=floor,facing=east]"));
    let toggles: u32 = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(2000);
    let t0 = std::time::Instant::now();
    for _ in 0..toggles {
        w.use_block(BlockPos::new(-1, 1, 0));
        for _ in 0..20 {
            w.tick();
        }
    }
    let dt = t0.elapsed().as_secs_f64();
    let ticks = toggles as f64 * 20.0;
    println!("{} ticks in {:.3}s = {:.0} ticks/s ({:.1} us per toggle)", ticks, dt, ticks / dt, dt * 1e6 / toggles as f64);
}

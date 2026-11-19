use redsim_core::data::data;
use redsim_core::pos::BlockPos;
use redsim_core::World;

fn s(t: &str) -> u32 {
    data().parse_state(t).unwrap()
}

fn main() {
    let mut w = World::new();
    let (ox, oz) = (0, 0);
    w.place_raw(BlockPos::new(ox, 0, oz), s("observer[facing=east]"));
    w.place_raw(BlockPos::new(ox - 1, 0, oz), s("stone"));
    for i in 0..8 {
        let x = ox - 1 - i;
        w.place_raw(BlockPos::new(x, -1, oz), s("stone"));
        w.place_raw(BlockPos::new(x, 0, oz), s("redstone_wire[east=side,west=side]"));
        w.place_raw(BlockPos::new(x, -1, oz + 1), s("stone"));
        w.place_raw(BlockPos::new(x, 0, oz + 1), s("sticky_piston[facing=up]"));
        w.place_raw(BlockPos::new(x, 1, oz + 1), s("slime_block"));
        w.place_raw(BlockPos::new(x, 2, oz + 1), s("stone"));
    }
    w.place_like_player(BlockPos::new(ox + 1, 0, oz), s("observer[facing=west]"));
    let n = std::env::args().nth(1).map(|a| a.parse().unwrap()).unwrap_or(200000);
    let t0 = std::time::Instant::now();
    for _ in 0..n {
        w.tick();
    }
    let dt = t0.elapsed().as_secs_f64();
    println!("{} ticks in {:.3}s = {:.0} ticks/s", n, dt, n as f64 / dt);
}

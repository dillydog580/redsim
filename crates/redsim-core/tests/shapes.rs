use redsim_core::data::data;
use redsim_core::pos::Dir;
use redsim_core::shape::{face_supports, Support};

/// The geometric face-support test must agree with the game's own precomputed
/// isFaceSturdy for every block state whose support shape is its collision shape.
#[test]
fn face_support_matches_game_for_all_states() {
    let d = data();
    let hook = d.hooks.iter().position(|h| h == "getBlockSupportShape").unwrap();
    let mut checked = 0;
    let mut bad = Vec::new();
    for (sid, info) in d.states.iter().enumerate() {
        let b = d.binfo(info.block);
        if b.hooks & (1 << hook) != 0 {
            continue;
        }
        let boxes = d.collision(sid as u32);
        for dir in Dir::ALL {
            for (ty, mask) in [(Support::Full, info.sturdy_full), (Support::Center, info.sturdy_center), (Support::Rigid, info.sturdy_rigid)] {
                let want = mask & (1 << dir.ordinal()) != 0;
                let got = face_supports(boxes, dir, ty);
                checked += 1;
                if want != got && bad.len() < 20 {
                    bad.push(format!("{} {:?} {:?}: game={} ours={}", d.format_state(sid as u32), dir, ty, want, got));
                }
            }
        }
    }
    assert!(bad.is_empty(), "{} checks, mismatches:\n{}", checked, bad.join("\n"));
}

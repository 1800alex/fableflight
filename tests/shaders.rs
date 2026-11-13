//! Parses and validates every shader without needing a GPU.
fn check(name: &str, src: &str) -> bool {
    match naga::front::wgsl::parse_str(src) {
        Err(e) => {
            eprintln!("--- {name}\n{}", e.emit_to_string(src));
            false
        }
        Ok(m) => match naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m) {
            Err(e) => {
                eprintln!("--- {name}\n{}", e.emit_to_string(src));
                false
            }
            Ok(_) => true,
        },
    }
}

#[test]
fn shaders_validate() {
    let common = include_str!("../src/shaders/common.wgsl");
    // Modules that draw the sky get sky.wgsl too, as in render.rs.
    let sky = include_str!("../src/shaders/sky.wgsl");
    let mut ok = check("noise3d", include_str!("../src/shaders/noise3d.wgsl"));
    ok &= check("resolve", include_str!("../src/shaders/resolve.wgsl"));
    ok &= check("detail", include_str!("../src/shaders/detail.wgsl"));
    for (name, src) in [
        ("lut", include_str!("../src/shaders/lut.wgsl")),
        ("terrain", include_str!("../src/shaders/terrain.wgsl")),
        ("buildings", include_str!("../src/shaders/buildings.wgsl")),
        ("plane", include_str!("../src/shaders/plane.wgsl")),
        ("post", include_str!("../src/shaders/post.wgsl")),
        ("particles_sim", include_str!("../src/shaders/particles_sim.wgsl")),
        ("particles_draw", include_str!("../src/shaders/particles_draw.wgsl")),
        ("roads", include_str!("../src/shaders/roads.wgsl")),
        ("trees", include_str!("../src/shaders/trees.wgsl")),
        ("inst", include_str!("../src/shaders/inst.wgsl")),
        ("vol", include_str!("../src/shaders/vol.wgsl")),
        ("ao", include_str!("../src/shaders/ao.wgsl")),
        ("taa", include_str!("../src/shaders/taa.wgsl")),
    ] {
        ok &= check(name, &format!("{common}\n{src}"));
    }
    for (name, src) in [("composite", include_str!("../src/shaders/composite.wgsl")), ("clouds", include_str!("../src/shaders/clouds.wgsl"))] {
        ok &= check(name, &format!("{common}\n{sky}\n{src}"));
    }
    assert!(ok, "shader errors above");
}

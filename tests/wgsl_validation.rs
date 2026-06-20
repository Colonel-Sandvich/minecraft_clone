use std::collections::BTreeSet;

use minecraft_clone::world::chunk::mesh::vertex_pulling::SHADER_SOURCE;
use naga::{front::wgsl, valid};

fn parse_vertex_pulling_shader() -> naga::Module {
    wgsl::parse_str(SHADER_SOURCE).expect("vertex_pulling.wgsl should parse")
}

#[test]
fn vertex_pulling_wgsl_validates() {
    let module = parse_vertex_pulling_shader();
    valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
        .validate(&module)
        .expect("vertex_pulling.wgsl should validate");
}

#[test]
fn vertex_pulling_bindings_match_render_layout() {
    let module = parse_vertex_pulling_shader();
    let bindings = module
        .global_variables
        .iter()
        .filter_map(|(_, variable)| variable.binding.as_ref())
        .map(|binding| (binding.group, binding.binding))
        .collect::<BTreeSet<_>>();

    let expected = BTreeSet::from([
        (0, 0),
        (0, 1),
        (0, 2),
        (0, 4),
        (0, 5),
        (0, 6),
        (0, 7),
        (0, 8),
        (1, 0),
        (1, 1),
        (1, 2),
    ]);

    assert_eq!(bindings, expected);
}

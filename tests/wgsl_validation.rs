use std::collections::BTreeSet;

use minecraft_clone::world::chunk::mesh::TERRAIN_SHADER_SOURCE;
use naga::{AddressSpace, ImageClass, ImageDimension, ScalarKind, TypeInner, front::wgsl, valid};
use wesl::{
    CompileOptions,
    resolver::VirtualResolver,
    syntax::{ModulePath, PathOrigin},
};

const TERRAIN_TABLES_SHADER_SOURCE: &str = include_str!("../assets/shaders/terrain_tables.wesl");
const TERRAIN_SHADING_SHADER_SOURCE: &str = include_str!("../assets/shaders/terrain_shading.wesl");
const MESH_SHADER_SOURCE: &str = include_str!("../assets/shaders/mesh_shader.wesl");

#[derive(Debug, PartialEq, Eq)]
struct ShaderResource<'a> {
    group: u32,
    binding: u32,
    name: &'a str,
    kind: ResourceKind,
}

#[derive(Debug, PartialEq, Eq)]
enum ResourceKind {
    Uniform,
    ReadOnlyStorage,
    Texture2dArrayFloat,
    Sampler,
}

fn shader_module_path(components: &[&str]) -> ModulePath {
    ModulePath::new(
        PathOrigin::Absolute,
        components
            .iter()
            .map(|component| (*component).to_string())
            .collect(),
    )
}

fn compile_shader(root: &str) -> String {
    let mut resolver = VirtualResolver::new();
    resolver.add_module(
        shader_module_path(&["shaders", "vertex_pulling"]),
        TERRAIN_SHADER_SOURCE.into(),
    );
    resolver.add_module(
        shader_module_path(&["shaders", "mesh_shader"]),
        MESH_SHADER_SOURCE.into(),
    );
    resolver.add_module(
        shader_module_path(&["shaders", "terrain_shading"]),
        TERRAIN_SHADING_SHADER_SOURCE.into(),
    );
    resolver.add_module(
        shader_module_path(&["shaders", "terrain_tables"]),
        TERRAIN_TABLES_SHADER_SOURCE.into(),
    );

    // Mirrors the WESL compilation settings Bevy uses for shader assets.
    let options = CompileOptions {
        imports: true,
        condcomp: true,
        visibility: false,
        ..Default::default()
    };

    wesl::compile(&shader_module_path(&["shaders", root]), &options, &resolver)
        .unwrap_or_else(|error| panic!("{root}.wesl should compile from WESL to WGSL: {error}"))
        .to_string()
}

fn parse_shader(root: &str) -> naga::Module {
    let source = compile_shader(root);
    wgsl::parse_str(&source).unwrap_or_else(|error| {
        panic!("compiled {root}.wesl should parse as WGSL: {error:?}");
    })
}

fn parse_vertex_pulling_shader() -> naga::Module {
    parse_shader("vertex_pulling")
}

#[test]
fn vertex_pulling_wesl_validates() {
    let module = parse_vertex_pulling_shader();
    valid::Validator::new(valid::ValidationFlags::all(), valid::Capabilities::empty())
        .validate(&module)
        .expect("vertex_pulling.wesl should validate");
}

#[test]
fn mesh_shader_wesl_validates() {
    let source = compile_shader("mesh_shader");
    assert!(
        source.contains("enable wgpu_mesh_shader;"),
        "mesh shader must keep the mesh-shader WGSL extension enabled"
    );
    let module = parse_shader("mesh_shader");
    valid::Validator::new(
        valid::ValidationFlags::all(),
        valid::Capabilities::MESH_SHADER,
    )
    .validate(&module)
    .expect("mesh_shader.wesl should validate");
}

#[test]
fn mesh_shader_lowers_to_spirv() {
    let module = parse_shader("mesh_shader");
    let info = valid::Validator::new(
        valid::ValidationFlags::all(),
        valid::Capabilities::MESH_SHADER,
    )
    .validate(&module)
    .expect("mesh_shader.wesl should validate before lowering");
    let options = naga::back::spv::Options {
        lang_version: (1, 4),
        ..Default::default()
    };
    let words = naga::back::spv::write_vec(&module, &info, &options, None)
        .expect("mesh_shader.wesl should lower to SPIR-V");
    assert!(!words.is_empty(), "SPIR-V output should not be empty");
}

#[test]
fn mesh_shader_declares_mesh_output_and_entry_points() {
    let module = parse_shader("mesh_shader");
    let mesh = module
        .entry_points
        .iter()
        .find(|entry| entry.name == "mesh")
        .expect("mesh_shader.wesl should declare a `mesh` entry point");
    assert_eq!(mesh.stage, naga::ShaderStage::Mesh);
    let info = mesh
        .mesh_info
        .as_ref()
        .expect("`mesh` should be a mesh-shader entry point");
    assert_eq!(info.topology, naga::MeshOutputTopology::Triangles);
    assert_eq!(info.max_vertices, 32 * 6);
    assert_eq!(info.max_primitives, 32 * 2);
    assert!(
        module
            .entry_points
            .iter()
            .any(|entry| entry.name == "fragment" && entry.stage == naga::ShaderStage::Fragment),
        "mesh_shader.wesl should reuse the fragment entry-point name"
    );
    assert!(
        !module
            .entry_points
            .iter()
            .any(|entry| entry.stage == naga::ShaderStage::Task),
        "this backend intentionally has no task shader"
    );
}

#[test]
fn mesh_shader_bindings_match_render_layout() {
    let module = parse_shader("mesh_shader");
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
    let bindings = shader_resources(&module)
        .iter()
        .map(|resource| (resource.group, resource.binding))
        .collect::<BTreeSet<_>>();
    assert_eq!(bindings, expected);
}

#[test]
fn vertex_pulling_bindings_match_render_layout() {
    let module = parse_vertex_pulling_shader();
    let bindings = shader_resources(&module)
        .iter()
        .map(|resource| (resource.group, resource.binding))
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

#[test]
fn vertex_pulling_resource_metadata_matches_render_layout() {
    let module = parse_vertex_pulling_shader();
    let resources = shader_resources(&module);

    // WESL's escape mangler prefixes imported declarations with the package
    // and module path. Keep the full qualified names exact so a rename or an
    // accidentally different binding source is caught. These names may need
    // updating deliberately when upgrading WESL; do not loosen them to suffix
    // matching just to make an upgrade pass.
    assert_eq!(
        resources,
        vec![
            ShaderResource {
                group: 0,
                binding: 0,
                name: "package_shaders__1terrain_shading__1view_proj",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 0,
                binding: 1,
                name: "package_shaders__1terrain_shading__1terrain_texture",
                kind: ResourceKind::Texture2dArrayFloat,
            },
            ShaderResource {
                group: 0,
                binding: 2,
                name: "package_shaders__1terrain_shading__1terrain_sampler",
                kind: ResourceKind::Sampler,
            },
            ShaderResource {
                group: 0,
                binding: 4,
                name: "package_shaders__1terrain_shading__1texture_layers",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 0,
                binding: 5,
                name: "package_shaders__1terrain_shading__1tint_colors",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 0,
                binding: 6,
                name: "package_shaders__1terrain_shading__1ao_brightness",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 0,
                binding: 7,
                name: "package_shaders__1terrain_shading__1emission_factors",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 0,
                binding: 8,
                name: "package_shaders__1terrain_shading__1terrain_visuals",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 1,
                binding: 0,
                name: "package_shaders__1terrain_shading_faces",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 1,
                binding: 1,
                name: "package_shaders__1terrain_shading__1chunk_origin",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 1,
                binding: 2,
                name: "package_shaders__1terrain_shading__1light_data",
                kind: ResourceKind::ReadOnlyStorage,
            },
        ]
    );
}

fn shader_resources(module: &naga::Module) -> Vec<ShaderResource<'_>> {
    let mut resources = module
        .global_variables
        .iter()
        .filter_map(|(_, variable)| {
            let binding = variable.binding?;
            Some(ShaderResource {
                group: binding.group,
                binding: binding.binding,
                name: variable
                    .name
                    .as_deref()
                    .expect("bound resource should be named"),
                kind: resource_kind(module, variable),
            })
        })
        .collect::<Vec<_>>();
    resources.sort_by_key(|resource| (resource.group, resource.binding));
    resources
}

fn resource_kind(module: &naga::Module, variable: &naga::GlobalVariable) -> ResourceKind {
    match (&variable.space, &module.types[variable.ty].inner) {
        (AddressSpace::Uniform, _) => ResourceKind::Uniform,
        (AddressSpace::Storage { access }, _) if *access == naga::StorageAccess::LOAD => {
            ResourceKind::ReadOnlyStorage
        }
        (
            AddressSpace::Handle,
            TypeInner::Image {
                dim: ImageDimension::D2,
                arrayed: true,
                class:
                    ImageClass::Sampled {
                        kind: ScalarKind::Float,
                        multi: false,
                    },
            },
        ) => ResourceKind::Texture2dArrayFloat,
        (AddressSpace::Handle, TypeInner::Sampler { comparison: false }) => ResourceKind::Sampler,
        _ => panic!(
            "unexpected resource type for {:?} at {:?}",
            variable.name, variable.binding
        ),
    }
}

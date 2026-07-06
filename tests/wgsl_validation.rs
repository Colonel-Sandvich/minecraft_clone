use std::collections::BTreeSet;

use minecraft_clone::world::chunk::mesh::vertex_pulling::SHADER_SOURCE;
use naga::{AddressSpace, ImageClass, ImageDimension, ScalarKind, TypeInner, front::wgsl, valid};

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

    assert_eq!(
        resources,
        vec![
            ShaderResource {
                group: 0,
                binding: 0,
                name: "view_proj",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 0,
                binding: 1,
                name: "terrain_texture",
                kind: ResourceKind::Texture2dArrayFloat,
            },
            ShaderResource {
                group: 0,
                binding: 2,
                name: "terrain_sampler",
                kind: ResourceKind::Sampler,
            },
            ShaderResource {
                group: 0,
                binding: 4,
                name: "texture_layers",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 0,
                binding: 5,
                name: "tint_colors",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 0,
                binding: 6,
                name: "ao_brightness",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 0,
                binding: 7,
                name: "emission_factors",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 0,
                binding: 8,
                name: "terrain_visuals",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 1,
                binding: 0,
                name: "faces",
                kind: ResourceKind::ReadOnlyStorage,
            },
            ShaderResource {
                group: 1,
                binding: 1,
                name: "chunk_origin",
                kind: ResourceKind::Uniform,
            },
            ShaderResource {
                group: 1,
                binding: 2,
                name: "light_data",
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

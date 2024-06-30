use bevy::math::Vec3;

pub const fn splat_xz(v: f32) -> Vec3 {
    Vec3 { x: v, y: 0.0, z: v }
}

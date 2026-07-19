#![feature(const_trait_impl)]

pub mod app;
pub mod audio;
pub mod block;
pub mod game_state;
pub mod input;
pub mod item;
pub mod light;
pub mod memory;
pub mod mob;
pub mod player;
pub mod quad;
pub mod textures;
pub mod ui;
pub mod util;
pub mod world;

pub use app::{AppPlugin, run};

# bevy-query-disjoint

> Keep query access disjoint; resolve overlap with filters, `ParamSet`, or separate systems.

## Why It Matters

Bevy validates component access when a system is initialized and panics when it cannot prove two parameters are disjoint. The borrow checker cannot help across separate `Query` parameters, and `With<A>` versus `With<B>` is not proof of disjointness — an entity can carry both markers. A system that only fails for unusual archetypes is worse: it passes normal play and panics in a corner case.

## Bad

```rust
// Panics at system initialization: an entity may be both Player and Enemy.
fn clamp_health(
    mut players: Query<&mut Health, With<Player>>,
    mut enemies: Query<&mut Health, With<Enemy>>,
) {
    // ...
}
```

## Good

Make the filters provably disjoint when the sets really are exclusive:

```rust
fn clamp_health(
    mut players: Query<&mut Health, (With<Player>, Without<Enemy>)>,
    mut enemies: Query<&mut Health, (With<Enemy>, Without<Player>)>,
) {
    // ...
}
```

Use `ParamSet` when both queries must see overlapping entities, and borrow one side at a time:

```rust
fn clamp_health(
    mut health: ParamSet<(
        Query<&mut Health, With<Player>>,
        Query<&mut Health, With<Enemy>>,
    )>,
) {
    let player_health = health.p0().single().map(|h| h.0).unwrap_or(0);
    for mut enemy in &mut health.p1() {
        enemy.0 = enemy.0.min(player_health);
    }
}
```

If the two passes do not need shared state, split them into separate systems and order them with a set instead.

## Project Notes

- `ParamSet` is already the pattern in `src/world/dimension/fluid.rs` and `src/game_test/mod.rs`; follow it rather than widening queries.
- Prefer `Single<&mut T, With<Active>>` / `Option<Single<...>>` over `query.single().unwrap()` for exactly-one entities. This repo uses that form heavily in `src/world/dimension`.
- When a system compiles but the schedule panics, check archetype overlap before changing anything else.

## See Also

- [bevy-schedule-sets](bevy-schedule-sets.md) - encode the order of the split systems
- [own-borrow-over-clone](own-borrow-over-clone.md) - borrow instead of cloning snapshots
- [anti-clone-excessive](anti-clone-excessive.md) - do not clone to dodge query conflicts

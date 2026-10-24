# bevy-io-networking

> Pick the pool by workload: Compute for this frame, AsyncCompute for later frames, Io for async IO, one dedicated thread for blocking sockets/DB.

## Why It Matters

Bevy gives you three task pools and a scheduler. Using the wrong one stalls a frame (work on the main thread), starves a pool (a blocking read parks an async-compute worker), or oversubscribes cores (rayon/Tokio alongside Bevy's pools). File writes and packet loops are not CPU parallelism; isolate them at the edge and drain results into the ECS.

## Decision Table

| Workload | Use | Pattern |
| --- | --- | --- |
| CPU work needed this frame (meshing, light propagation) | `ComputeTaskPool` | `par_iter` / `par_splat_map`, join before returning |
| CPU work spanning frames | `AsyncComputeTaskPool` | `Task<T>` + `check_ready` each frame |
| Async file/asset IO | `IoTaskPool` | `spawn(async move { ... }).detach()` or keep the `Task` |
| Blocking socket / DB loop | one dedicated thread | bounded channel in, channel out, drained by a system |
| Game networking/replication | a networking crate | keep transport at the boundary, feed ECS messages |

## File Writes

```rust
use bevy::tasks::IoTaskPool;

fn save_settings(settings: Settings) {
    IoTaskPool::get()
        .spawn(async move {
            if let Err(error) = std::fs::write(SETTINGS_PATH, settings.encode()) {
                bevy::log::error!(%error, "failed to save settings");
            }
        })
        .detach();
}
```

For whole-world saves keep using the `ChunkStore`/`ChunkTaskPool` path; do not add a second save executor. Bound the number of in-flight writes, and coalesce "latest settings" instead of queueing every edit. For settings, write a temp file and rename it into place so a crash cannot leave a half-written save.

## Sockets and Packets

Bevy has no built-in game networking. For a hand-rolled LAN game, keep the socket off the ECS:

```rust
use std::sync::Mutex;

#[derive(Resource)]
struct NetReceiver(Mutex<std::sync::mpsc::Receiver<Packet>>);

fn spawn_net_thread(mut commands: Commands) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let socket = match std::net::UdpSocket::bind("0.0.0.0:0") {
            Ok(socket) => socket,
            Err(error) => {
                bevy::log::error!(%error, "failed to bind socket");
                return;
            }
        };
        let mut buf = [0u8; 1200];
        loop {
            match socket.recv_from(&mut buf) {
                Ok((len, from)) => {
                    if tx.send(Packet::decode(&buf[..len], from)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    bevy::log::warn!(%error, "socket read failed");
                    break;
                }
            }
        }
    });
    commands.insert_resource(NetReceiver(Mutex::new(rx)));
}

fn drain_packets(receiver: Res<NetReceiver>, mut messages: MessageWriter<NetPacket>) {
    let receiver = receiver.0.lock().expect("net receiver poisoned");
    for packet in receiver.try_iter() {
        messages.write(NetPacket(packet));
    }
}
```

- `std::sync::mpsc::Receiver` is `!Sync`, hence the `Mutex`. For multiple producers or `select`, `crossbeam-channel` is the usual choice, but it is a new dependency.
- Use a **bounded** outbound channel and drop or coalesce stale snapshots. Never queue unbounded on a network hiccup.
- Set `set_read_timeout` or use non-blocking mode so the reader can observe shutdown.
- Sending: a system writes to an outbound channel; the socket thread owns the socket. Never call `send_to` from a system.

## Networking Crates

Check Bevy 0.20 compatibility before choosing one:

- `bevy_replicon` — ECS replication layer (server-authoritative entities/components).
- `lightyear` — client-server transport, netcode, and replication.
- `renet` / `naia` — transport/protocol crates you build on.
- `matchbox` — WebRTC for browser/native peer-to-peer.

Keep the chosen crate at the boundary. Do not let transport types leak into gameplay systems; convert packets to ECS messages at the edge.

## Project Notes

- `TursoChunkStore` already owns a Tokio runtime for the `turso-store` feature. Do not reuse it for networking or general file IO; keep it inside the storage adapter.
- Settings, screenshots, and exports: `IoTaskPool`. Chunk saves: the existing `ChunkTaskPool` + `ChunkStore`.
- Never spawn a thread per frame or per packet. One long-lived reader thread/task per socket, one writer path.
- Log write failures with context (see [err-context-chain](err-context-chain.md)); a dropped save is data loss.
- A bounded outbound queue turns a network hiccup into backpressure instead of unbounded memory growth.

## See Also

- [bevy-task-pools](bevy-task-pools.md) - choose the pool for CPU work
- [bevy-blocking-boundary](bevy-blocking-boundary.md) - keep blocking work out of systems
- [bevy-commands-deferred](bevy-commands-deferred.md) - channel results are not visible until the next frame
- [err-context-chain](err-context-chain.md) - surface IO failures with context

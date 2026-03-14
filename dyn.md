# Dynamic Actors via RuntimeHandle

## Context

Actor graphs in `tokio-actor` are intentionally static today: topology is fixed at `GraphBuilder::build()` time. That remains true after this change. Static graphs stay the way users define the initial system, and runtime dynamism is layered on top through `tokio-otp::RuntimeHandle`.

The goal of this pass is narrower than "fully dynamic graphs":

- add actors at runtime
- remove actors at runtime
- let existing actors discover and send to runtime-added actors
- keep restart-stable actor refs and supervision semantics

This does **not** attempt to mutate `GraphBuilder`, rewrite static links in place, or add dynamic ingress management yet.

## Design

### 1. Keep static peers static

`ActorContext::peers` stays the fast path for links declared in `GraphBuilder`.

- `ctx.peer()` and `ctx.send()` continue to operate only on predeclared links
- no locks or registry lookups are added to the existing static send path

Runtime discovery is additive:

- `ctx.registry()` exposes an optional shared registry
- `ctx.send_dynamic(actor_id, envelope)` checks static peers first, then falls back to registry lookup

### 2. Add `ActorRegistry` to `tokio-actor`

Dynamic discovery belongs in `tokio-actor`, because it is fundamentally about stable actor references and mailbox rebinding.

`ActorRegistry` is a concurrent directory keyed by actor id. Each entry stores enough internal actor metadata to create a fresh `ActorRef` for:

- external callers through `RuntimeHandle::actor_ref()`
- actors calling `ctx.send_dynamic(...)`

The registry holds restart-stable bindings, so callers keep the same semantics they already get from `ActorRef` today:

- if an actor restarts, refs rebind automatically
- if an actor is known but currently stopped, sends return `ActorNotRunning`
- if an actor id is not registered, lookup fails immediately

Initial API surface:

- `ActorRegistry::new()`
- `contains(actor_id) -> bool`
- `actor_ids() -> Vec<String>`
- `actor_ref(actor_id) -> Option<ActorRef>` for non-actor callers
- `deregister(actor_id) -> Result<(), RegistryError>`

Registry seeding/registration stays actor-owned instead of exposing mailbox internals:

- `RunnableActor::register_with(&ActorRegistry) -> Result<(), RegistryError>`

That keeps `MailboxBinding`, `MailboxRef`, and observability internals private to `tokio-actor`.

### 3. Build dynamic actors in `tokio-actor`, supervise them in `tokio-otp`

The runtime add/remove API belongs in `tokio-otp`, but the actual "turn an `ActorSpec` into a runnable, mailbox-bound actor" logic belongs in `tokio-actor`.

Reason:

- `RunnableActor::run_until()` already owns mailbox binding, blocking task handling, and actor lifecycle behavior
- `ActorSpec` factories and mailbox/observability internals are not public, and should stay that way

So `tokio-actor` grows a dynamic actor factory:

- `ActorSet::dynamic_factory() -> RunnableActorFactory`
- `RunnableActorFactory::actor(spec) -> RunnableActor`
- `RunnableActorFactory::actor_with_peer_ids(spec, registry, peer_ids) -> Result<RunnableActor, RegistryError>`

The factory captures the graph-level runtime settings:

- graph name for observability
- mailbox capacity
- envelope size limit
- blocking task concurrency limit
- blocking shutdown timeout

### 4. Seed static actors into the registry

When `SupervisedActors::build_runtime()` builds a `Runtime`, it:

1. creates an `ActorRegistry`
2. seeds every static actor into the registry via `RunnableActor::register_with`
3. injects that registry into every static `RunnableActor`
4. stores both the registry and a `RunnableActorFactory` in the runtime's dynamic state

This makes the initial static graph discoverable to:

- newly added runtime actors
- existing static actors calling `ctx.send_dynamic(...)`

### 5. Runtime API

`RuntimeHandle` grows the dynamic actor control surface:

- `add_actor(spec, options) -> Result<ActorRef, DynamicActorError>`
- `remove_actor(actor_id) -> Result<(), DynamicActorError>`
- `actor_ref(actor_id) -> Option<ActorRef>`

`DynamicActorOptions` controls:

- supervisor restart policy
- shutdown policy
- optional restart intensity override
- optional initial linked peers by id

The peer list is resolved once at actor creation time using the registry. This gives dynamic actors the same ergonomic `ctx.send("peer", ...)` path for known dependencies, while `ctx.send_dynamic(...)` remains available for late-bound targets.

## Message flow

### Add actor

1. Build a `RunnableActor` from the runtime's `RunnableActorFactory`
2. Resolve configured peer ids from the shared registry
3. Inject the shared registry into the actor
4. Register the actor in the shared registry
5. Wrap the actor in a `ChildSpec`
6. Submit the child to the underlying supervisor with `add_child`
7. Return the actor's stable `ActorRef`

If supervisor insertion fails, the registry entry is rolled back.

### Remove actor

1. Ask the supervisor to remove the child by id
2. On success, deregister the actor id from the shared registry

That preserves the supervisor as the source of truth for child lifecycle and shutdown behavior.

## Non-goals in this pass

- mutate `GraphBuilder` or `Graph`
- dynamically rewrite static links for already-running actors
- add/remove ingress points at runtime
- support runtime actor insertion for manually assembled `Runtime::new(...)` values that were not built from `SupervisedActors`

Manual `Runtime::new(...)` can still exist, but dynamic actor operations return an explicit unsupported error because the runtime lacks the actor graph metadata needed to construct new actors safely.

## Implementation phases

### Phase 1: `tokio-actor`

- add `registry.rs` with `ActorRegistry` and `RegistryError`
- extend `ActorContext` with `registry()` and `send_dynamic()`
- extend `RunnableActor` with:
  - `actor_ref()`
  - `register_with(&ActorRegistry)`
  - `set_registry(ActorRegistry)`
- add `RunnableActorFactory`
- add `ActorSet::dynamic_factory()`

### Phase 2: `tokio-otp`

- add `DynamicActorError`
- add `DynamicActorOptions`
- store optional dynamic runtime state inside `Runtime` / `RuntimeHandle`
- seed registry + actor registry access in `SupervisedActors::build_runtime()`
- implement `RuntimeHandle::add_actor`, `remove_actor`, and `actor_ref`

### Phase 3: tests

- dynamic actor can be added and receives messages
- static actor can reach dynamic actor through `send_dynamic`
- dynamic actor can reach static actors through configured peer ids and/or `send_dynamic`
- removing a dynamic actor deregisters it
- runtimes created through plain `Runtime::new(...)` report dynamic support as unavailable

## Verification

1. `cargo +nightly fmt`
2. `cargo test -p tokio-actor -p tokio-otp`
3. `cargo +nightly clippy --workspace --all-targets --all-features`
4. stage newly added files
5. `just ci`

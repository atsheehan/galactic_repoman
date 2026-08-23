# Step 3 (detailed) — Backend-agnostic input (winit keyboard → gilrs gamepad → Steam Input)

Keyboard, gamepad, and eventually Steam Input, behind one module that emits a single correct
event stream. The module owns device-state tracking so that **the game never has to defend
against a stream that lies** — and so a polled backend and an evented one are
indistinguishable from the other side.

## Context

`galactic_repoman` currently opens a window and draws a static colored triangle
(Step 1, landed). `app.rs` runs a `ControlFlow::Wait` event loop that draws only when
something asks it to; `renderer.rs` issues `draw(3, 1, 0, 0)` against three vertices baked
into `triangle.vert` with no uniforms or push constants; the only input handled anywhere is
a hardcoded `Escape` match arm inside `App::window_event`.

**This step reorders and reshapes the parent plan.** The parent plan
(`plans/scaffolding-vulkan-steamos-game.md`) had input as Step 4, arriving after audio and
built *only* on the Steam Input action API. Two changes, both user-directed:

1. **Input moves ahead of audio.** Input is what is being built next, and audio has not
   started. Audio becomes Step 4; the Steam Input backend becomes Step 5.
2. **Steam Input becomes one backend behind an abstraction, not the only path.** The game
   must run standalone, outside Steam, so `winit` (keyboard) and `gilrs` (gamepad) are the
   baseline and Steam Input is layered in later as a swappable backend. No `winit`, `gilrs`,
   or `steamworks` type may appear in the game's own code.

**Exit criteria for this step:** holding the left/right movement binding spins the rendered
triangle, on keyboard (3c) and on a gamepad stick or d-pad (3d), with no stuck-spin after
alt-tab or controller unplug.

## The update model

The whole design is organized around one requirement from the user:

```
game_state_a + event -> game_state_b
```

A pure fold. The game does not poll input, and the update function has no side effects and
no hidden clock. Everything the game reacts to arrives as an event.

**Events-only is the chosen shape** — no `is_down()` / `axis()` polling API on the input
module. The objection to events-only is normally that the game has to reconstruct held-state
from edges and gets it wrong (a `Released` lost to focus change leaves the triangle spinning
forever). This plan answers that objection by **moving the bookkeeping into the input module
instead of removing it**, per the user's framing: the module tracks which sources are
currently down and, on any event that could invalidate that picture, checks its own map and
emits whatever corrective events are needed. The game never has to defend against a lying
event stream, because the module does not emit one.

### These are not two copies of the same state

Worth being explicit, because "the module tracks state AND the game tracks state" sounds
redundant and isn't:

| | The input module tracks | The game tracks |
| --- | --- | --- |
| What | *Device* truth: which physical keys / buttons / axes are down right now | *Intent*: `spin: f32`, `angle: f32`, `should_quit: bool` |
| Why | So the emitted event stream is edge-correct and gap-free | Because it is the game's own model, produced by the fold |
| Lifetime | Reset by focus loss, disconnect, backend swap | Owned by `GameState`, only ever changed by `update` |

The module's map is a backend concern that exists solely to compute edges. It is never read
by the game and never exposed. `GameState` remains the single source of truth for anything
the game or renderer cares about.

### The one addition the model needs: time

`state + event -> state` cannot on its own produce continuous rotation. `Pressed(SpinLeft)`
sets a direction, but something has to advance the angle while nothing is happening. So the
clock enters the same stream as an event:

```rust
/// Everything `update` folds over. The clock is an event so the update step stays a
/// single pure fold with no side channel.
pub enum Event {
    Input(input::InputEvent),
    Tick { dt: Duration },
}

pub fn update(state: GameState, event: Event) -> GameState { ... }
```

`app.rs` per frame: drain the input events, append exactly one `Tick`, fold the lot.

```rust
let state = input
    .drain()                       // Iterator<Item = InputEvent>
    .map(Event::Input)
    .chain(std::iter::once(Event::Tick { dt }))
    .fold(state, game::update);
```

> **Alternative considered:** keep `update(state, input_event)` pure-input and add a separate
> `advance(state, dt)`. It keeps "input" and "time" in distinct types, but splits the fold in
> two and means every caller must remember to call both in the right order. The single-stream
> version is one fold, matches the user's framing literally, and makes a replayable event log
> fall out for free (a recorded stream of `Event`s reproduces a run exactly). Going with the
> single stream; easy to split later if the mixed enum grates.

`dt` comes from an `Instant` delta clamped to a ceiling (say 100 ms), so a run that stalls on
a swapchain rebuild or a breakpoint resumes without teleporting the triangle through a full
rotation. Fixed-timestep accumulation is a later concern — variable `dt` is correct for a
spinning triangle and the `Tick` event shape does not change if it is added.

## Module structure

Per `CLAUDE.md` (`src/renderer.rs`, not `src/renderer/mod.rs`; functional style preferred):

```
src/
  input.rs            # public API: Action, Axis, InputEvent, Input. Owns source→action
                      # resolution, edge detection, and the corrective-event logic.
    input/
      keyboard.rs     # winit WindowEvent -> SourceEdge
      gamepad.rs      # gilrs pump        -> SourceEdge   (3d)
      bindings.rs     # the source→action/axis binding table (data, not logic)
      (steam.rs)      # Steam Input poll  -> SourceEdge   (Step 5)
  game.rs             # GameState + update(state, event) -> GameState. No backend types.
  app.rs              # drives: pump input -> fold -> request redraw. Modified.
  renderer.rs         # render(&mut self, angle: f32). Modified.
  shaders/triangle.vert  # + push constant. Modified.
```

The backend submodules are deliberately dumb: they translate one backend's notion of "this
thing went down / went up / moved to 0.34" into a common `SourceEdge` and hand it to
`input.rs`. All the interesting logic — refcounting, deadzones, corrective events — lives
once in `input.rs` and is therefore shared by every backend, including Steam Input.

## Public API

```rust
/// A digital thing the game reacts to. No backend type ever appears here.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Action {
    Quit,
}

/// A continuous −1.0..=1.0 input. Keyboard bindings contribute ±1.0; sticks contribute
/// their own magnitude. The game sees one number and does not care which produced it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Axis {
    Spin,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum InputEvent {
    Pressed(Action),
    Released(Action),
    AxisMoved(Axis, f32),
}

pub struct Input { /* private: sources, axis values, pending queue */ }

impl Input {
    pub fn new() -> anyhow::Result<Self>;

    /// Feed one winit window event. Queues whatever it implies.
    pub fn handle_window_event(&mut self, event: &WindowEvent);

    /// Pump the polled backends (gilrs now, Steam Input later). Call once per frame.
    pub fn poll(&mut self);

    /// Take everything queued since the last call.
    pub fn drain(&mut self) -> impl Iterator<Item = InputEvent> + '_;
}
```

Splitting `Action` (digital) from `Axis` (analog) is not incidental — it is the same split
Steam Input's action sets use (`GetDigitalActionData` / `GetAnalogActionData`), so Step 5
maps onto it directly instead of fighting it.

## How the module guarantees a correct stream

Internally an action or axis can be driven by several **sources** at once:

```rust
enum Source {
    Key(PhysicalKey),
    Button(GamepadId, gilrs::Button),
    Stick(GamepadId, gilrs::Axis),
}
```

### Digital: refcount the sources, emit only on 0↔1

`HashMap<Action, HashSet<Source>>`. A source going down inserts; a source going up removes.
`Pressed` is emitted **only** on the empty→non-empty transition and `Released` **only** on
non-empty→empty.

This is the generalized form of the edge case that motivated the design. Without it, binding
both `A` and `Left` to the same action means releasing `A` while still holding `Left` emits a
spurious `Released` and the triangle stops mid-hold. It covers cross-backend overlap for free:
holding the key *and* the d-pad, then releasing one, correctly emits nothing.

### Analog: resolve, then emit only on change

Each `Axis` has a set of contributing sources with their current values (keyboard bindings
contribute a constant ±1.0 while held). Each frame the module resolves them to a single
value, and emits `AxisMoved` **only if** it differs from the last emitted value by more than
an epsilon — otherwise a resting stick floods the queue with identical events every frame.

Resolution rule: sum the contributions and clamp to `[-1.0, 1.0]`. Holding both left and
right cancels to 0.0, which is the conventional and least-surprising behaviour.

### The corrective cases

Each of these is a place a naive backend translation produces a wrong or missing event, and
each is handled once inside `input.rs`:

| Situation | What the backend does | What the module does |
| --- | --- | --- |
| **Window loses focus** (alt-tab) | winit sends `Focused(false)` and **no key-up** — the key-up goes to whoever has focus now | On `Focused(false)`, drop every `Source::Key(_)`, then emit `Released` for each action that emptied and `AxisMoved` for each axis that changed. *This is the stuck-spin bug; without it the triangle spins forever after alt-tab.* |
| **Key auto-repeat** | winit re-sends `KeyboardInput` with `repeat: true` while held | Ignore `repeat == true` outright. (Field confirmed present on `winit 0.30`'s `KeyEvent`.) A held key must produce exactly one `Pressed`. |
| **Gamepad unplugged mid-hold** | gilrs sends `EventType::Disconnected` and nothing else | Drop every source belonging to that `GamepadId`, emit the resulting `Released`/`AxisMoved`. |
| **Stick resting on the deadzone edge** | gilrs streams `AxisChanged` continuously | For digital bindings, hysteresis: enter at 0.5, leave at 0.4. Without the gap, a thumb resting near 0.5 emits a `Pressed`/`Released` storm. For analog bindings, a plain radial deadzone with rescaling so the usable range still reaches 1.0. |
| **Same key pressed twice with no release** | Can happen across a focus round-trip | The 0↔1 rule suppresses it — no special case needed. |
| **App starts with a key already held** | Unknowable; no backend reports it | Start empty. "Nothing is held" is the safe reading; the first real key-up is ignored because its source was never inserted. |

### Why this design makes the Steam Input backend easy

Steam Input is **polled**, not evented — Step 5 reads `GetDigitalActionData` /
`GetAnalogActionData` once a frame and gets a level, not an edge. That is normally an
awkward mismatch with an event-based API.

Here it is already solved: the module *already* converts polled values into edges, because
that is exactly what it does to gilrs axes. `input/steam.rs` reads each action's current
level, diffs it against the module's stored source state, and the shared refcount logic emits
the edges. The polled backend and the evented backend produce an identical event stream
because neither of them computes edges — `input.rs` does.

This is the concrete payoff of putting state tracking in the module rather than the client.

### Testability

Both halves are pure state machines with no window, no GPU, and no device:

- `input.rs` — feed a synthetic sequence of `SourceEdge`s, assert the emitted `InputEvent`s.
  The valuable ones are the corrective cases: *press A, press Left, release A → exactly one
  `Pressed` and no `Released`*; *press A, lose focus → exactly one `Released`*; *press A,
  lose focus, regain focus → nothing further*.
- `game.rs` — `update` is `(GameState, Event) -> GameState`, so a test is a `fold` over a
  literal event vector and an assertion on the result.

## Bindings

Movement binds to **physical** keys (`PhysicalKey::Code(KeyCode::KeyA)`), not logical ones,
so WASD stays under the same fingers on AZERTY/Dvorak. `Quit` binds to logical `Escape`
(a named key means the same thing on every layout). The current `Escape` arm in
`App::window_event` is deleted in favour of `Action::Quit`.

| Axis / action | Keyboard | Gamepad (3d) |
| --- | --- | --- |
| `Axis::Spin` | `KeyA`/`ArrowLeft` → −1.0, `KeyD`/`ArrowRight` → +1.0 | Left stick X (analog); d-pad left/right (±1.0) |
| `Action::Quit` | `Escape` (logical) | `Start` — or leave gamepad-unbound for now |

The table lives in `input/bindings.rs` as data. Runtime rebinding and config files are out of
scope for this step, but keeping bindings as a table rather than as `match` arms is what makes
that a later data change instead of a rewrite.

## Changes to existing code

### Clearing out the Step 1 diagnostics

`app.rs` carries two pieces of instrumentation added while chasing the black screen on
relaunch. Both come out first, before anything is added, so the input work lands on a smaller
file:

- **The heartbeat** — `HEARTBEAT`, `started`, `last_heartbeat`, `resume_count`, `occluded`,
  `log_heartbeat`, and the `WaitUntil` re-arm in `about_to_wait`. It existed to tell "parked
  in `Wait`, nothing asked us to draw" apart from "wedged inside `render`", a distinction that
  stops being interesting once the loop draws continuously.
- **`log_window_event`** — the per-event `debug` log added to look for a hidden `Occluded`
  event suppressing the first frame. It also becomes actively harmful in 3d: a resting
  gamepad stick would flood it.

> **Trade-off, stated once.** The black screen on relaunch was never explained, and this
> removes the instrumentation that would characterize it if it recurs. Accepted deliberately —
> the diagnostics are recoverable from git history, and `Renderer::status()` still reports
> draw attempts, presents, and swapchain state on request. Two related leftovers are *not*
> covered by this step and stay as they are: the magenta `CLEAR_COLOR` and the per-frame
> `render_future.wait(None)` stall.

### `app.rs` — the loop shape changes

- **`ControlFlow::Wait` → `ControlFlow::Poll`**, but only from 3b onward — 3a is still a
  static scene and has no reason to leave `Wait`. See *Power* below: this is not a blanket
  switch to `Poll`, it is `Poll` while animating and `Wait` while idle or occluded.
- `about_to_wait` becomes: `input.poll()` → compute `dt` → fold → `window.request_redraw()`.
- `window_event` forwards to `input.handle_window_event(&event)` and keeps its existing
  window-lifecycle arms (`Resized`, `Occluded`, `RedrawRequested`, …). `CloseRequested` still
  exits directly — it is a window-manager event, not a player action.
- Exit is driven by `state.should_quit` after the fold, so `Action::Quit` and the window
  close button converge on one path.

### `renderer.rs` and the shader — the triangle has to rotate

The vertex shader currently bakes three positions with no uniforms, so there is nothing to
rotate. A push constant is the smallest change that fixes that:

```glsl
// triangle.vert
layout(push_constant) uniform Push {
    float angle;   // radians
} pc;

// in main():
float s = sin(pc.angle), c = cos(pc.angle);
gl_Position = vec4(mat2(c, s, -s, c) * positions[gl_VertexIndex], 0.0, 1.0);
```

> **Decision: no aspect correction.** Clip space is square and the window is not, so the
> triangle will visibly stretch as it turns. Accepted for now — the goal of this step is to
> see that input has an effect, and coordinate normalization is a separate concern that
> belongs with a real 2D camera/projection rather than bolted onto a diagnostic triangle.
> Worth knowing so the stretching is not later mistaken for a bug.

**What a push constant is.** Semantically it *is* a uniform — the shader declares it with the
`uniform` keyword and reads it identically. The difference is how the data arrives. A uniform
buffer means allocating a `VkBuffer`, writing it, building a descriptor set, and binding that
set — and updating it per frame means either one buffer per frame-in-flight or manual
synchronization, because the GPU may still be reading the previous frame's copy. A push
constant is written **inline into the command buffer** by `vkCmdPushConstants`, so each frame's
recorded commands carry their own copy: no buffer, no descriptor set, no synchronization. The
cost is size — the Vulkan spec guarantees only **128 bytes** (`maxPushConstantsSize`), and AMD
(so RADV on SteamOS) commonly reports exactly that. It suits a handful of floats: a transform,
a time value, an index. One `float` per frame is precisely the case it exists for.

- **`renderer/pipeline.rs`** needs no structural change.
  `PipelineDescriptorSetLayoutCreateInfo::from_stages(&stages)` reflects the push constant
  range straight out of the SPIR-V, so the existing layout construction picks it up. *Verify
  at implementation time* that the reflected range is non-empty rather than assuming it.
- **`renderer.rs`** — `render()` takes the angle, and between `bind_pipeline_graphics` and
  `draw` calls
  `builder.push_constants(self.pipeline.layout().clone(), 0, vs::Push { angle })`
  (signature confirmed against vulkano 0.35: `push_constants<Pc: BufferContents>(Arc<PipelineLayout>, u32, Pc)`;
  `vulkano_shaders` generates the `Push` struct). Signature becomes `render(&mut self, angle: f32)`
  for now; it grows into a scene/view struct once there is more than one thing to draw. The
  renderer must not take `&GameState` — that would put game types back inside the renderer.

### Power: what `Poll` costs, and what actually mitigates it

**Vsync is already the throttle; do not add sleeps.** The swapchain is created with
`PresentMode::Fifo`. Once every swapchain image is queued for presentation,
`acquire_next_image` blocks until the compositor releases one at vblank, so the loop parks
inside that call at the display refresh rate. An artificial `thread::sleep` would be strictly
worse: coarse granularity, added frame-pacing jitter, and it guesses at an interval the
compositor already knows exactly.

Two things do need handling, and the first is a genuine busy-loop:

- **Occluded or zero-extent must fall back to `Wait`.** `render()` returns early when the
  window reports a zero extent — and an early return means *nothing blocks*. Minimize the
  window under `Poll` and the loop spins at 100% CPU with no frames to pace it. The existing
  `Occluded` handler is the hook; it must now also set the control flow.
- **Idle should not redraw.** With spin at zero the scene is static, but `Poll` would still
  redraw every vblank forever, which matters on a handheld. `update` already knows whether
  the state changed, so drive the choice off that: `Poll` while animating, `Wait` when not.
  Any input event wakes the loop and flips it back.

The per-frame `render_future.wait(None)` stall stays as it is. It burns a CPU-side wait every
frame rather than pipelining, which is real but bounded — it blocks rather than spins, and
`Fifo` caps how often it happens. Revisit before there is a real scene.

## Phasing

Five chunks, each independently runnable and each failing in one identifiable way. The
ordering isolates the graphics change from the timing change from the input change, so a
triangle that does not move can only be one thing at a time.

- **3a — Rotate by a fixed amount.** Still a static render, still `ControlFlow::Wait`, no
  input and no clock. Add the push constant to `triangle.vert`, plumb it through
  `renderer.rs`, and hand it a hardcoded angle (say 0.5 rad).

  Proves the shader, pipeline-layout reflection, and `push_constants` call in isolation. If
  the triangle is not visibly tilted, the problem is the push constant path and nothing else.
  Also the natural moment to delete the heartbeat and `log_window_event`.

- **3b — Rotate over time.** Introduce `game.rs`: `GameState { angle }`, the `Event` enum with
  only a `Tick { dt }` variant, and `update`. `app.rs` computes `dt` from an `Instant` delta
  (clamped), folds one `Tick` per frame, and requests a redraw. Switch to `Poll` here, along
  with the `Wait`-when-occluded fallback.

  Triangle spins at a constant rate. Proves the loop, the timing, and the fold. Still no
  `input` module at all.

- **3c — Keyboard, naive.** `input.rs` + `input/keyboard.rs` + `input/bindings.rs`; `Action`,
  `Axis`, `InputEvent`; `Event::Input` joins `Event::Tick`. Left/right set the spin direction.

  **Deliberately no corrective logic** — no focus handling, no repeat filtering, no
  multi-source refcounting. Expect the stuck-spin bug on alt-tab; that is 3e's job. One thing
  to get right anyway: store held sources as a `HashSet<Source>` per action from the start,
  even while the only rule is insert/remove. 3e then adds logic on top of an existing data
  structure instead of rewriting one.

- **3d — Gamepad.** Add `gilrs = "0.11"` and `input/gamepad.rs`. `Gilrs::new()` in
  `Input::new()`; `poll()` drains `next_event()` in a loop; map `ButtonPressed`/`ButtonReleased`/
  `AxisChanged` to `SourceEdge`. Analog stick drives `Axis::Spin` proportionally.

  Verified against the gilrs 0.11.2 source: `Gilrs::next_event() -> Option<Event>`,
  `Event { id, event: EventType, time }`, `EventType::{ButtonPressed(Button, Code),
  ButtonReleased(..), AxisChanged(Axis, f32, Code), Connected, Disconnected, Dropped}`.
  `EventType::Dropped` must be ignored.

  **No change to `game.rs` or to any public input type should be required by 3d** — if one is,
  the abstraction is wrong, and that is the signal to fix it before Step 5.

- **3e — Edge cases.** The corrective table, in full: focus loss clearing held keys, repeat
  filtering, 0↔1 refcounting across multiple sources, gamepad disconnect, deadzone hysteresis.
  Plus the `input.rs` and `game.rs` unit tests, which are what make this phase checkable
  rather than merely plausible.

  **Not optional.** 3c and 3d ship known-broken input; this is the phase that makes it
  correct, and the alt-tab regression is the reason the module exists.

- **Step 5 — Steam Input.** Gets its own plan doc. Two things to carry forward:

  - Steam Input and gilrs will both enumerate the same physical pad, so a naive "run both"
    double-counts every press. The likely rule is *Steam Input replaces gilrs when the Steam
    client initializes; keyboard always stays live* — to be settled in that step, not assumed
    here.
  - Steam Input's action-set model is a superset of `Action`/`Axis`, so the manifest can be
    authored to match this enum rather than the other way round.

## Verification

Each phase has its own gate; nothing below is checked before the phase that introduces it.

1. **3a:** `cargo run` shows a *tilted* triangle, held still. Resize and close still work, no
   validation errors. `app.rs` no longer mentions the heartbeat or `log_window_event`.
2. **3b:** the triangle spins at a steady rate on its own. Resize while spinning → no stutter,
   no validation errors. **Minimize the window and watch CPU** — it must go to roughly idle,
   not one core pinned. That is the `Wait`-when-occluded fallback, and it is the single
   easiest thing to get wrong in this step.
3. **3b:** with the triangle static (once spin is input-driven in 3c), the process should not
   be redrawing every vblank — confirm the loop returns to `Wait` when nothing is animating.
4. **3c:** hold `A`/`Left` → spins counter-clockwise; hold `D`/`Right` → clockwise; release →
   stops. Both at once → stationary. `Escape` exits via `Action::Quit`; the window close
   button still exits.
5. **3c (expected failure):** alt-tab while holding a key and the triangle keeps spinning.
   Confirm it, so 3e has a reproduction to fix rather than a hypothesis.
6. **3d:** gamepad stick spins proportionally — a half-deflected stick spins at about half
   rate; d-pad spins at full rate. Keyboard still works alongside it.
7. **3e:** the alt-tab case from (5) now leaves the triangle stationary and responsive to the
   next press. Hold `A`, also press `Left`, release `A` while still holding `Left` → the spin
   does not stutter or stop. **Unplug the pad mid-spin** → the triangle stops rather than
   spinning forever; replug → works again.
8. **3e:** `cargo test` — the `input.rs` state-machine cases and the `game.rs` fold cases.

The triangle stretching as it turns is expected throughout and is not a defect; see the
aspect-correction decision above.

## Risks / call-outs

- **The occluded busy-loop is the sharpest edge in this step.** Under `Poll`, a window with a
  zero extent makes `render()` return early, and an early return blocks on nothing — the loop
  spins flat out with no frames to pace it. It will not show up on a desktop unless someone
  minimizes the window, and it is exactly the kind of thing that first surfaces as "the Deck
  gets hot". Test it explicitly (verification 2), do not reason about it.
- **Removing the heartbeat gives up an unexplained bug's instrumentation.** The black screen
  on relaunch was never root-caused. Accepted, per the trade-off note above, but if it returns
  the first move is to restore that logging from git history rather than re-derive it.
- **gilrs on SteamOS/Sniper** reads `/dev/input/event*` via udev; permissions and the runtime
  container are the usual failure point, and it is untested here. Nothing before 3d depends on
  it, so 3a–3c are unaffected.
- **3c and 3d ship deliberately-broken input.** Sequencing the edge cases last is what makes
  each phase individually debuggable, but it means two phases where alt-tab leaves the
  triangle spinning. The risk is 3e being treated as polish and deferred; it is not optional,
  and the `HashSet<Source>` shape in 3c is the hedge that keeps it additive.
- **Deadzone values are guesses** until tested on real hardware. 0.5/0.4 hysteresis for
  digital and a radial deadzone around 0.15 for analog are starting points, not findings.
- **`Tick` in the same enum as input events** is the one part of the update model most likely
  to be regretted. If `Event` accumulates non-input variants (network, timers, audio
  completion), revisit whether input deserves its own fold.

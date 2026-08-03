# freddie

## Commits

Commit after every change, small and atomically, without being asked. Each logical change is its own commit.

## Refactor docs

This section is extremely important. A frequent source of frustration is deviations from this protocol. Take it very seriously and frequently refresh your memory on how to write planning documents. 99% of our time is spent iterating on planning documents, so it is extremely important that you do this correctly.

- The primary way we plan things is through documents in the `refactor/` folder.
- A doc plans changes to its own repo only. Work that spans figaro and freddie is two docs, one per repo, each self-contained for its half and referencing the other's public interface as a dependency; a figaro doc never specifies edits to a freddie crate, and a freddie doc never specifies edits to figaro.
- Move a `refactors/pending` doc to `refactors/past` when we will not work on it in the future.
- Each doc must, at all times that we are actively working on it, conform to several standards:
  - It should describe what we are building. Do not discuss how we came to a conclusion, or what we are not building. Do not narrate your thought process. Do not discuss what has already landed.
  - It should have enough information for a new agent, with no context, to completely implement the feature **without making any important decisions.** All decisions are made as part of the planning document. Do not take shortcuts.
  - Stubs, hand-waving, and "sketch this later" are disallowed. The planning document must be comprehensive: write out the real types, functions, call sites, and before/after snippets. If we do not actually write the stuff out, it is impossible to know whether the implementation is real or just fantasy.
  - Every struct, enum, and other data type must be written out in full. The data layout is the most important thing to review; a prose description of the shape is not a substitute for the actual fields and variants.
  - Every interface must be explicit: the functions, their signatures, who calls them, and what they return. The end-user experience must be written out too (what the user does, what they see, what layer they end in), not left as a gloss on the code.
  - If a step involves a complicated algorithm, procedural macros, or recursion, write out the generated or expanded code. The expansion is what we review; the generator is not a substitute for it.
  - All changes should have before and after snippets. New functions, new structs, etc. should be written out in advance.
  - If you need to have an additional scratch pad, you may — but do not do that work within the freddie repository, and do not check it in. Do not "write tests" for work that is still under active discussion.
  - Paragraphs of text are useless. Prefer code snippets.
  - Follow all coding standards listed below.
- The docs may have two parts (which may be split across multiple docs):
  - An overall discussion of the problem being worked on, and
  - An ordered list of changes. Each change should be self-contained and independently shippable. It should be ordered such that early changes are prefactors that make the actual, consequential change as easy as possible.
- When we are discussing a change, always try to identify independently shippable changes. If these changes are guaranteed (or nigh thereunto), then we can ship them as a prefactor, and thus limit the complexity of the actual change (and planning document).
- When a doc is not being actively worked on, it may become stale. That is okay. It should be updated to not be stale when we start working on it in the future. In other words, if we are working on `A`, and `B` depends on `A`, we do not need to keep `B` up to date unless it's part of the discussion.
- If a refactor is too large and should be broken up into smaller steps (e.g. "Chrome extension that informs mercury of changes" -> "Mercury receives events on a port" + "Chrome extension that sends events"), let the user know, and do so. The files should be "conceptually different".
- While we are iterating on a pending doc, stay in the doc. Start implementing only when the user gives explicit permission to implement. "Looks good," edits to the doc, "go on," "continue," or further planning discussion are not permission. If there is any ambiguity about whether implementation has been authorized, do not start implementing.

## Implementing a refactor doc

Implementation begins only after the explicit permission above. Starting an implementation is not a commitment to finish it no matter what the code turns out to say. The doc was written so that no important decisions are left to the implementer, so when you hit something the doc did not anticipate, the decision is still the user's to make, not yours to improvise.

- Stop and ask as soon as the doc stops matching the code. A step that assumed a type, a call site, or an ownership arrangement that is not there is a defect in the doc, and the fix goes into the doc first.
- The signal to stop is complexity, above all. If a step that read as small turns out to pull in a redesign, a new shared-state primitive, a new trait, or a change to a crate the doc never mentioned, that is exactly the case to raise rather than absorb quietly. Say what exploded and what the options are, and let the user pick.
- `git stash` the half-finished work while we settle it, if that leaves a cleaner tree to discuss against. Say what you stashed and what state it is in, so nothing is lost while the doc is being corrected.
- Do not paper over the gap by choosing the easy version, leaving a TODO, or narrowing the step so it fits. Those hide the decision instead of surfacing it.

## Workarounds a deferred solution would obviate

Some problems already have their fix: a long-term solution we have planned and deliberately not built yet. The virtual HID backend is the standing example (`refactors/past/cgevent-vs-hid.md` frames it as the known upgrade behind the `intercept` seam): secure input blinding the tap, re-posted output re-entering the shared event stream, and the rest of the CGEventTap class all stop existing when it lands.

When a bug is in that position, raise it with the user before writing a workaround, every time. Most likely we do not want to solve the bug yet: the patch is code the deferred solution deletes, and it tends to spread — state, special cases, and tests shaped around a problem that is scheduled to disappear. This is a general rule, not a virtual HID special case: whenever a long-term but currently deferred solution would obviate the issue, be careful about adding patches that solution would throw away. Name the deferred solution, say what the workaround would cost, and let the user decide whether the bug is worth patching now.

## Shared state and interior mutability

`Arc`, `Rc`, `Mutex`, `RwLock`, `Cell`, `RefCell`, `OnceCell`/`OnceLock`, `lazy_static`, `thread_local!`, and atomics are almost always the wrong reach in freddie. The model is a pure function of state and event running on one thread; the state lives in one place, and a handler that wants a value already holds it. So when a design proposes any of these, three things happen every time:

- Question whether it is needed at all. Most of the time the value it is trying to share is already reachable without it, and the primitive is papering over a design that put the value in the wrong place.
- Default to the version that does not need it. Write that version first and only fall back to shared mutable state when the non-shared version is genuinely, provably impossible — not merely more work.
- Raise it with the user, every single time, before it goes into a planning doc or into code. There are no exceptions to this. Name the primitive, say what it is sharing and why the non-shared version does not work, and wait for the user's decision.

The preferred way to move data between threads is a channel whose sender is freely `Send` and cloneable while the receiver stays pinned to one thread. Sending an event to the thread that owns the state beats reaching into that state across a lock. If a design reaches for `Arc<Mutex<_>>`, the first question is what channel would carry that data instead.

Ambient state the program itself owns belongs on the root struct, not in a `static` or a `thread_local!`. A process-wide counter, a next-id source, anything a dispatch reads or advances: make it a field on the root model and thread it to where it is used, so the value it hands out is a function of state. A `static NEXT: AtomicU64` is the shape to avoid: it is ambient, it makes the dispatch that reads it impure, and it is usually `Sync` only to satisfy `static` rather than because two threads touch it.

This is distinct from state the outside world owns. The front app and a window's frame are seeded at construction and kept current by events, under the idempotence rule above; they are external truth mirrored into the model, not ambient state the program invented. The test is who mints the value. If the program mints it, it is a field on root. If the OS owns it, it arrives as an event.

## unwrap, unreachable, Infallible

`unwrap`, `expect`, `unreachable!`, `panic!`, `todo!`, `unimplemented!`, and `Infallible` (including a `Result<T, Infallible>` or any other type-level claim that a failure case cannot occur) are almost always the wrong reach in freddie. The model makes impossible states unrepresentable: a handler that needs a value already holds it at the type level, and a branch that cannot run is not an arm. So when a design proposes any of these, three things happen every time:

- Question whether it is needed at all. Most of the time the type is wrong. An `Option` or `Result` the body will always unwrap should not have been optional. A match arm that is always unreachable is a state the handler should never have been handed. An `Infallible` error type is a `Result` that should not be a `Result`.
- Default to the version that does not need it. Fix the types so the success path is the only path the compiler allows. Write that version first and only fall back to a panic or an infallibility claim when the non-panicking version is genuinely, provably impossible — not merely more work.
- Raise it with the user, every single time, before it goes into a planning doc or into code. There are no exceptions to this. Name the construct, say what invariant it is asserting and why the type system cannot express that invariant, and wait for the user's decision.

The preferred alternative is the same structural fix the typed path already does for layers. A state a binding cannot be reached in is not an arm that panics; it is a value the handler is never handed. An `Option` that is always `Some` at a call site is a non-optional field or a narrower type. A `Result` whose `Err` is `Infallible` returns the `Ok` payload directly.

This is distinct from total handling of values the outside world owns. An OS callback, a parse of a user-supplied path, a socket read: those can fail for reasons the program does not control, and the response is a typed error, a skip, or an event that reports the failure — never an unwrap of something the OS controls (see `docs/platform-apis.md`). A panic must not cross an FFI boundary.

Tests may `expect` with a reason that names an invariant the test itself established (a fixture it built, an env the harness sets). Production code is not a test fixture.

## Booleans

`bool` is almost always the wrong reach in freddie. Prefer an enum whose variants name the states. A boolean is two anonymous cases; an enum makes those cases part of the type, so call sites match on meaning rather than on `true`/`false`, and a third state is a new variant instead of a second flag or a comment.

So when a design proposes a `bool` field, parameter, or return type, three things happen every time:

- Question whether it is needed at all. Most of the time the two cases have names (`Enabled`/`Disabled`, `Open`/`Closed`, `Foreground`/`Background`) and belong as variants of an enum, not as `true`/`false` on a field called `is_*`.
- Default to the enum. Write that version first and only fall back to a `bool` when the value is genuinely a pure yes/no with no domain names worth carrying — not merely because a flag is shorter to type.
- Raise it with the user, every single time, before it goes into a planning doc or into code. There are no exceptions to this. Name the `bool`, say what the two cases mean, and wait for the user's decision if an enum is not the obvious replacement.

This is the same maintainability rule as "make impossible states unrepresentable." A field that only exists when a flag is set is not `flag: bool` plus `payload: Option<T>`; it is `Option<T>`, or an enum with a payload-bearing variant. Two booleans that cannot both be true are not two fields; they are one enum.

## Tests

The standard for the model is exhaustive: every key in every reachable state, asserting exactly what dispatch produces. The model is a pure function of state and event, so the full table is checkable and doubles as documentation of the keymap. Not all of it exists yet; new bindings should extend toward it rather than test only the happy path.

## Where a binding leaves you

Every binding decides what layer it ends in, and the decision follows from what the user is expected to do next. A new binding that does not answer this is unfinished.

- If the action is one you would plausibly do again right away, stay in the layer. Walking tmux's windows and refreshing Chrome repeat, so they stay.
- If it is a choice rather than something you repeat, leave. Placing a window and jumping to a numbered tmux window are each one decision, so they go home (`and_go_home`). Nav's app-choosers leave too, into the in-app layer.
- If what follows the action is typing, the layer it leaves for is typing. Anything that puts a cursor in a text field qualifies: Chrome's `l` focuses the address bar, and claude.ai's `n` opens a new chat in its prompt box. Both end in `to_typing`, because a command layer would swallow what the user typed next.

## Running mercury

Mercury is the live keyboard remapper on this machine: while it is stopped, the keyboard behaves the way macOS would. There is exactly one at a time (`refactors/past/single-instance.md`), so a second cannot run alongside it.

Stopping and restarting it is what the verbs are for, and they work. Say what you are doing to it, and leave one running when you are done.

- `mercury` starts one detached and says its pid, or says which one is already running. `mercury start` is the same thing spelled out.
- `mercury restart` replaces the running one, which is what a rebuild wants. `--force` destroys the old one rather than asking it to quit.
- `mercury stop` ends it through the model, so the modifiers a command layer swallowed are reopened.
- `mercury status` reports the running one and its pid; `mercury logs` follows the log. Neither touches the process.

`bacon restart` does the rebuild and the replacement together, so an edited binding goes live without touching a window.

The event socket reaches a running daemon without touching the process: connect to `127.0.0.1:3883` and send a frame, then read the dispatch record it produced out of the log.

## Logs

mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run.

The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards. Every line is one flat JSON object: `pid`, `timestamp`, `level`, `target`, and the record's own fields (`message` and whatever the call site logged) beside them, in the order they were logged. So `jq` reads it, and `mercury logs` renders it rather than parsing text.

It holds one record per dispatched event, carrying the event, the effects it produced, and the resulting state, plus each key emitted, each app foregrounded, and the raw frontmost-app changes `freddie_app_nav` observed.

`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane with `mercury logs`, which follows the file and shows records at `info` and above; `mercury logs --level debug` widens that.

`mercury logs` leaves the state out. It is the whole model under `Debug`, which is most of a dispatch record and is wanted while something is being debugged; `mercury logs --include-state` puts it back, and `mercury logs --json` gives the records as stored.

Every record carries the pid of the process that wrote it, because a client verb and the daemon both append to the one file. `pid` is always the writer; a field naming some other process says which, as `stop`'s `daemon=` does.

## Nothing is printed

`println!`, `eprintln!`, `print!`, `eprint!`, and `dbg!` do not appear in this codebase, and a new one is a mistake. Everything mercury says goes through `tracing`, so the log file is the whole record of a run rather than the part that did not go to a terminal. The terminal is a `tracing_subscriber` layer exactly as the file is.

A client verb's level is its audience:

- `info!` is the verb's answer. It reaches stdout, and there is one per invocation.
- `warn!` and `error!` are problems the user has to see. They reach stderr.
- `debug!` is what the verb did along the way. Only the file keeps it.

The daemon is different: its terminal is its log in full, filtered by `--log-level`.

Three things stay unrouted, because none of them is mercury's own output. clap writes `--help`, `--version`, and parse errors itself and exits. `tail`, under `mercury logs`, writes the file's own contents, which tracing would append back into the file being followed. Tests print for whoever is reading the test run.

## Best Practices for Handlers and Freddie Apps

- `state.handle` is pure, and the one exception is creating timers. It reads the event and the state it was handed, writes state, and returns effects. It never reads the outside world: no querying the window server, no asking the OS which app is frontmost, no reading a file or a socket. If a handler needs the id of the focused window, that id is already a field on the state, put there by an earlier event.
- Anything the outside world knows and a handler needs arrives as an event first. A subscriber observes the change, sends an event, and the handler records it in state. That is what makes a dispatch reproducible from `(state, event)` alone, and what keeps the model testable as a table.
- So state the outside world owns and the model consumes — the frontmost app, the front tab's URL, a window's frame, the focused element's selected text — is synced, never read on demand. A watcher observes the source, its events keep the mirrored field current, and the field is seeded at construction. An effect that asks the OS at the moment a binding fires, with the answer dispatched back as a follow-up event, keeps `handle` pure but is still the wrong shape: the value should already have been true in the model before the key went down. On-demand asking is reserved for a source with no observation channel at all, and each such read is a named exception (`Copied::FrontTabUrl`, whose reporting extension may simply not be connected), not a pattern to extend.
- A per-app fact (each app retains its own selection, Chrome its own front tab) is synced as the whole map, not as the value at the focus point: a background app's entry only changes while its observer is watching anyway, a missing entry is the honest "not reported yet", and the value at the focus point becomes a lookup joined with the already-mirrored front app instead of a re-seed on every switch.
- A mirror's event carries what its watcher knows instantly, nothing more. Never enrich a latency-critical event with a slow read: the activation event gates which bindings exist, an AX attribute read is bounded only by its messaging timeout, and coupling them makes a hung app delay the keymap. The slow fact arrives as its own report from its own source, and the model is honest in the gap between them.
- The effect side is dumb. `perform_effect` and the platform code under it carry out exactly what the payload says and decide nothing. They do not read state, do not consult the outside world to fill in a missing argument, and do not branch on anything but the effect's own variant.
- So the effect payload carries everything performing it needs. If foregrounding a window needs its id, the id is in the payload rather than looked up at performance time. An effect that would have to go find something is a sign the handler dropped information it already had.
- An event that reports state is idempotent: applying it twice lands where applying it once does. It assigns, replaces, or removes, and it never accumulates. `set_front_app` assigns the app, a window's frame is overwritten, a closed window is removed. A counter, a toggle, or an append would be wrong.

  This is what makes the boot ordering safe. Every watcher is installed before any seed is read (see `refactors/past/seed-at-construction.md`), so a change happening in that window arrives twice: once in the seed the model is constructed with, and once as the event the watcher queued. Chrome comes forward, the snapshot already says Chrome, the queued `Foreground(Chrome)` dispatches into a model that agrees, and nothing moves. The other ordering loses the change entirely, so this is the ordering, and idempotence is its price.

- A handler names a type parameter for everything it does not inspect, so one function binds at every place in the tree that can reach what it needs. The win is that the handler is reusable across those places and easier to test: a table drives it from the shallowest node that reaches the state, and the same test covers every deeper binding. Name a concrete type only where the body actually reads it.
  - The path is a bound, not a concrete type. `P: IntoAncestor<MercuryPath<'a>>` when the body consumes or mutates the root; `P: HasAncestor<MercuryPath<'a>>` when it reads an ancestor by shared ref and keeps using its own node. Never a concrete `LayerPath` or `AppLayerPath`. `HasAncestor`/`IntoAncestor` reach the root from any depth, so the one handler binds at home, in-app, and every level between; a `.parent().parent()` chain with a hardcoded hop count is what this replaces. (A leaf handler that only reads the root still consumes its node: dispatch hands the node by value, and `needless_pass_by_value` flags one that is merely borrowed, so it takes `IntoAncestor` and reads through the `&mut`.)
  - The event is a bound, `E` with `_ev: &E`, whenever the body does not read the trigger, so the same handler binds to a `KeyPress`, the menu bar's `Quit`, or a timer firing.
  - The node's data is a bound, `D` on `Node<P, D>`, whenever the body only reaches `node.parent`, so a derived level, which carries data rather than `()`, binds the handler too.
- Never construct a thing in an invalid or blank state and then fire an immediate message to correct it. If the correct initial state is known at construction, set it there. This is not just about freddie events: a channel send, a queued task, a deferred effect, anything that "fixes it up on the next tick" is the same mistake. The status item is created showing `Mercury::BOOT_TITLE`, not created blank and handed its title over the title channel; the boot layer's name is known before the model that would send it exists, so it belongs in the constructor. An immediately-fired message to reach a valid state is a sign the construction is in the wrong place: move what the message would set into the thing being built. The exception is a value the outside world owns and can change (the front app, a window frame): that is seeded at construction and also arrives as an event, per the idempotence rule above, and the event is not a fixup but the ongoing truth.
- A `freddie_*` crate is shared, so a decision in one is justified by the crate's own constraints, never by the disposition of a consumer. mercury, figaro, and whatever comes next all inherit it, so "figaro is a personal tool" can settle a choice in the figaro repo but says nothing about a `freddie_*` crate. When `freddie_virtual_hid` reuses Karabiner's driver instead of shipping one, the reason is the Apple-gated DriverKit entitlement the crate would otherwise have to carry; the cost, that every consumer needs Karabiner installed, is the crate's to state plainly, not one consumer's to wave away. A justification that names a consumer is a sign the decision or the reason for it is in the wrong place.

## Wrapping an operating system API

`docs/platform-apis.md` is what the `freddie_*` crates do when they hold something the OS gave them: which traits to claim and which to refuse, where `Drop` belongs, how a C callback reaches its state, and what the main thread is for. Read it before writing a new one or changing how an existing one holds a resource.

## laserbeam vs bind

`laserbeam` is the typed mutable path into a single-owner state tree: resolve to the active leaf, `get`/`get_mut` on the focused node, walk up with `into_parent` / `HasAncestor` / `IntoAncestor`. It knows nothing about triggers, handlers, or dispatch.

`bind` is the binding layer over that tree: `#[derive(Bind)]`, trigger → handler mappings, `Dispatch`, and (behind `check`) the accumulate half that walks live binds for collisions. It sits on laserbeam paths; it is not a place for state-tree mechanics.

Do not put anything in `bind` unless it is specifically about binding handlers to triggers, or about checking that those bindings are well-formed. Path navigation, resolve, ancestor walks, projections, parent chains, state-tree structure — that is laserbeam. A design that wants a new primitive and is not sure which crate it belongs in defaults to laserbeam when it is about the tree, and only reaches for bind when it is about the binding of a handler. If the proposed change would still make sense in a program that has a state tree and no key bindings at all, it does not belong in `bind`.

## Coding standards

- Maintainability is the most important standard. And that specifically means one thing: make impossible states unrepresentable and use the correct underlying representation or building blocks. Prefer enums over booleans (see Booleans above). If a field is not used when a flag is one way or the other, use an `Option` or a sum type, not a `bool` plus a spare field.
- If we have to do extra refactoring work to maintain the above, we should do the extra work. If we need to refactor large parts of freddie in order to have the right building blocks, then we will do that.
- Prefer the structurally correct solution to the easy one, even when the easy one is fast. A method that performs well but fits nothing around it, adding ambient state or composing poorly with how the rest of the system hands work around, is not worth its local simplicity. Find the version that reuses the seams the model already has and generalizes to the next problem, and do the work to reach it; the right structure is what lets us build far more complicated things on top. What `freddie_overlay` used to do — GCD into a `thread_local` panel table — is the anti-pattern: easy and prompt, but ambient and sharing no mechanism with any other main-thread hand-off. The structured version it uses now, a channel drained on the main loop's `on_wake` and woken on send, is more code and is correct, and it is prompt too, so speed is never the reason to take the shortcut.
- If we need a more performant, but less idiomatic impl, then create a newtype/struct/enum that encapsulates the ugly complexity but exposes an idiomatic API.
- If a comment provides no more information than one would get by reading the code, do not include the comment.
- A comment should not describe what wasn't done, ESPECIALLY if "we didn't do x" is more indicative of the fact that we either previously discussed doing X or in a previous iteration of a planning doc, you suggested doing X.
- A comment must not describe what lives elsewhere unless a reader of this file has a concrete reason to expect it here. A doc comment names what the thing is and stops.
- Be very cautious when adding state to the model. Every field is something every dispatch must keep true and every test must account for, so before adding one, check whether the value is already derivable from fields that exist or whether the event can be handled without remembering it.
- State that does exist on the model is as self-contained and general-purpose as possible: one field owning the whole mechanism behind a named type, not a scatter of parallel fields. `jk: DeviceSequence` beats `j_pressed`, `k_pressed`, and friends twice over; the type owns the legal combinations, so impossible states are unrepresentable and call sites match on meaning, and `DeviceSequence` is a general mechanism the next sequence reuses. The type has no connection to the actual keys being pressed: which keys form the sequence is the caller's configuration, the field name carries the specific use, and nothing named `Jk` appears in the type.
- State lives on the node whose behavior it implements, encapsulated behind a type with named states. The root is not a grab-bag: a struct of loose flags serving several unrelated mechanisms is the anti-pattern, and a `#[expect(clippy::struct_excessive_bools)]` is a confession, not a waiver. Sequence memory (a hold is open, an up is owed a swallow) is a named enum owned by its mechanism. Root placement needs a stated reason (the state must survive layer changes, or serves every layer); "the gate happens to run at the root" is not one, and machinery that exists only to protect one layer's behavior belongs to that layer.
- A responsibility shared by every layer binds once on the shared struct above them, never once per layer. The converse holds too: only universal responsibilities hoist. A row each layer decides for itself stays in the layer, even when several layers currently agree.
- Bind tables are organized by handler: every row feeding one handler is adjacent, one device's arm beside the other's. Never organized by device class.
- Overlay cards: one txt file per device per layer, the title naming the device, changed in the same commit as the binds they describe.
- In JavaScript, a discriminated union takes exactly one form: `{ kind: "Type.Variant", value: T }`. The tag is always `kind`, its value is the dotted `Type.Variant` name, and the payload is always the single `value` field (never inline fields, never a bare variant name). Every variant that shares a `Type` prefix belongs to the same union, so `Type.` is how you read off which union a value is in.
- No polling; wake on events. An idle system costs nothing: with no work it is asleep, not surfacing on a timer to check. Work arrives by waking whatever the consumer is parked on — the channel it `recv`s, or the OS wait it blocks in when that wait cannot be selected on (the main thread inside the AppKit run loop), which the producer wakes directly rather than polling beside. A loop that wakes every N milliseconds to look for work is polling even when it is dressed as a timeout or a run-loop slice, and that N is a latency floor paid in power for the life of the process. Delete it rather than shrink it, and prefer a wake that cannot be lost if it races the wait, so no timeout backstop is needed. `select!` or a woken channel is the shape; a bare timeout is a last resort the code justifies.
- Never rely on discipline what we can enforce with newtypes.
- Custom traits are generally to be avoided. Prefer a concrete type, an enum, a plain function, or a standard-library trait (`From`/`Into`, `Default`, the iterator traits) over introducing a trait of our own. A trait earns its place when several types genuinely implement it or it marks a real abstraction boundary; a trait with one implementor, reached for to make a generic infer or to fold a single call site's boilerplate, is the case to avoid. The exact line is not sharp yet, so when a design introduces a trait, question it the way the shared-state primitives are: say what it buys over a concrete type, and default to the version without it.

### Wrapping an operating system API

`docs/platform-apis.md` is what the `freddie_*` crates do when they hold something the OS gave them: which traits to claim and which to refuse, where `Drop` belongs, how a C callback reaches its state, and what the main thread is for. Read it before writing a new one or changing how an existing one holds a resource.

## Handlers are bound where they are valid

A handler should only be bound in a state in which it is valid, and its signature should carry exactly what it is entitled to use. The consequences:

- A handler that matched is valid to call. The machinery — triggers, the claim gate, which node the row sits on — decides whether a handler runs; the body must not re-check preconditions the tree already encodes. A handler that opens by testing whether it should have been called is bound on the wrong node or behind the wrong trigger.
- A handler does not clean up unrelated state. When every handler carries the same boilerplate arm (the 28 identical `Invalidated => (vec![], c)` arms that became `if_not_invalidated`), the arm is the caller's job, not the handlers'.
- The signature is the entitlement. A bind-row handler receives its live path, because that is the only state it can legitimately act on; it does not receive a maybe it must unwrap. Posts are the one shape that receives `AscendState`, because they run whether or not the node survived, and what to do about a dead node is genuinely per-post meaning.
- Branching on a mode flag inside a handler is the same defect one level up: the modes should be nodes (or derived substates), and the handler bound only in the mode where it applies.
- A handler is named for what it does, never for how it is triggered. The trigger already lives in the bind row, and one handler binds to different keys on different devices, so a key name in the handler is wrong on every other row that uses it. `n_home` (laptop `n`, but AltIns `h` on the Kinesis) is the canonical offense; its name is `escape_then_home`, the action it performs.

## Audits

When told to audit, the deliverable is the whole class fixed everywhere, not the instance that was quoted. Sweep every file the standard touches before reporting done; the failure mode is the user opening the most obvious place and finding the problem still there. An audit that only edits what was pointed at is not an audit.

## Coding standards: nits

- Rust enums should take one of two forms: `enum Foo { NoData }` or `enum Foo { NamedStruct(Struct) }`, and not `Tuple(A, B)` or `Curlies { foo: Bar }`. `Tuple((A, B))` is appropriate, though.
- The map `entry` API is encouraged. Prefer `map.entry(k).or_insert(...)`, `or_default`, `and_modify`, or a match on `Entry` over a separate `contains_key` / `get` / `get_mut` plus `insert` when both reading and writing a slot.

# freddie

## Commits

Commit after every change, small and atomically, without being asked. Each logical change is its own commit.

## Refactor docs

This section is extremely important. A frequent source of frustration is deviations from this protocol. Take it very seriously and frequently refresh your memory on how to write planning documents. 99% of our time is spent iterating on planning documents, so it is extremely important that you do this correctly.

- The primary way we plan things is through documents in the `refactor/` folder.
- Move a `refactors/pending` doc to `refactors/past` once its work is implemented and tested, or we have decided that the work is not worth doing.
- Each doc must, at all times that we are actively working on it, conform to several standards:
  - It should describe what we are building. Do not discuss how we came to a conclusion, or what we are not building. Do not narrate your thought process. Do not discuss what has already landed.
  - It should have enough information for a new agent, with no context, to completely implement the feature **without making any important decisions.** All decisions are made as part of the planning document. Do not take shortcuts.
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

The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards. It holds one record per dispatched event, carrying the event, the effects it produced, and the resulting state on a single line, plus each key emitted, each app foregrounded, and the raw frontmost-app changes `freddie_app_nav` observed.

`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane with `mercury logs`, which follows the file and shows records at `info` and above; `mercury logs --level debug` widens that.

Every record carries the pid of the process that wrote it, because a client verb and the daemon both append to the one file. `pid=` is always the writer; a field naming some other process says which, as `stop`'s `daemon=` does.

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
- The effect side is dumb. `perform_effect` and the platform code under it carry out exactly what the payload says and decide nothing. They do not read state, do not consult the outside world to fill in a missing argument, and do not branch on anything but the effect's own variant.
- So the effect payload carries everything performing it needs. If foregrounding a window needs its id, the id is in the payload rather than looked up at performance time. An effect that would have to go find something is a sign the handler dropped information it already had.
- An event that reports state is idempotent: applying it twice lands where applying it once does. It assigns, replaces, or removes, and it never accumulates. `set_front_app` assigns the app, a window's frame is overwritten, a closed window is removed. A counter, a toggle, or an append would be wrong.

  This is what makes the boot ordering safe. Every watcher is installed before any seed is read (see `refactors/past/seed-at-construction.md`), so a change happening in that window arrives twice: once in the seed the model is constructed with, and once as the event the watcher queued. Chrome comes forward, the snapshot already says Chrome, the queued `Foreground(Chrome)` dispatches into a model that agrees, and nothing moves. The other ordering loses the change entirely, so this is the ordering, and idempotence is its price.

## Coding standards

- Maintainability is the most important standard. And that specifically means one thing: make impossible states unrepresentable and use the correct underlying representation or building blocks. If a field is not used when a boolean is true/false, use an option, for example.
- If we have to do extra refactoring work to maintain the above, we should do the extra work. If we need to refactor large parts of freddie in order to have the right building blocks, then we will do that.
- If we need a more performant, but less idiomatic impl, then create a newtype/struct/enum that encapsulates the ugly complexity but exposes an idiomatic API.
- If a comment provides no more information than one would get by reading the code, do not include the comment.
- A comment should not describe what wasn't done, ESPECIALLY if "we didn't do x" is more indicative of the fact that we either previously discussed doing X or in a previous iteration of a planning doc, you suggested doing X.
- In JavaScript, a discriminated union takes exactly one form: `{ kind: "Type.Variant", value: T }`. The tag is always `kind`, its value is the dotted `Type.Variant` name, and the payload is always the single `value` field (never inline fields, never a bare variant name). Every variant that shares a `Type` prefix belongs to the same union, so `Type.` is how you read off which union a value is in.
- Never poll and loop. Always select! or the like, unless there is a specific, unavoidable need.
- Never rely on discipline what we can enforce with newtypes.

### Coding standards: nits

- Rust enums should take one of two forms: `enum Foo { NoData }` or `enum Foo { NamedStruct(Struct) }`, and not `Tuple(A, B)` or `Curlies { foo: Bar }`. `Tuple((A, B))` is appropriate, though.

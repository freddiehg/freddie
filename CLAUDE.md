# freddie

## Commits

Commit after every change, small and atomically, without being asked. Each logical change is its own commit.

## Refactor docs

Move a `refactors/pending` doc to `refactors/past` once its work is implemented and tested.

Small, easy-to-state items that do not need a doc of their own go in `refactors/pending/todos.md`, one line each. A big or subtle item still gets its own doc.

## Tests

The standard for the model is exhaustive: every key in every reachable state, asserting exactly what dispatch produces. The model is a pure function of state and event, so the full table is checkable and doubles as documentation of the keymap. Not all of it exists yet; new bindings should extend toward it rather than test only the happy path.

## Logs

mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run.

The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards. It holds one record per dispatched event, carrying the event, the effects it produced, and the resulting state on a single line, plus each key emitted, each app foregrounded, and the raw frontmost-app changes `freddie_app_nav` observed.

`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane: `tail -f ~/Library/Logs/mercury/mercury.log`.

## Coding standards

- Maintainability is the most important standard. And that specifically means one thing: make impossible states unrepresentable and use the correct underlying representation. If a field is not used when a boolean is true/false, use an option, for example.
- If we have to do extra refactoring work to maintain the above, we should do the extra work.
- If we need a more performant, but less idiomatic impl, then create a newtype/struct/enum that encapsulates the ugly complexity but exposes an idiomatic API.

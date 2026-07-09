# freddie

## Commits

Commit after every change, small and atomically, without being asked. Each logical change is its own commit.

## Refactor docs

Move a `refactors/pending` doc to `refactors/past` once its work is implemented and tested.

## Logs

mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run; it is the only place the logs go (the terminal only gets the startup lines and a fatal keyboard error).

One record per dispatched event, carrying the event, the effects it produced, and the resulting state on a single line. `RUST_LOG` sets the level, defaulting to `info`:

- `info` — the per-dispatch records, and startup/kill.
- `debug` — adds each key emitted, each app foregrounded, and `freddie_app_nav`'s raw frontmost-app changes.

Run with more detail: `RUST_LOG=debug cargo run -p mercury`. Watch it live from another pane: `tail -f ~/Library/Logs/mercury/mercury.log`.

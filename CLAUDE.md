# freddie

## Commits

Commit after every change, small and atomically, without being asked. Each logical change is its own commit.

## Refactor docs

Move a `refactors/pending` doc to `refactors/past` once its work is implemented and tested.

## Logs

mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run.

The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards. It holds one record per dispatched event, carrying the event, the effects it produced, and the resulting state on a single line, plus each key emitted, each app foregrounded, and the raw frontmost-app changes `freddie_app_nav` observed.

`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane: `tail -f ~/Library/Logs/mercury/mercury.log`.

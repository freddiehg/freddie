---
title: Logging
sidebar_position: 8
---

# Logging

`mercury` writes to `~/Library/Logs/mercury/mercury.log`, always, appending across runs, and always down to `debug` whatever the terminal was asked for. One record per dispatched event carries the event, the effects it produced, and the resulting state, so a run is reconstructable afterwards.

```bash
mercury logs                 # records at info and above
mercury logs --level debug   # widen that
```

Every record carries the pid of the process that wrote it, because a client verb and the daemon both append to the one file. `pid=` is always the writer; a field naming some other process says which, as `stop`'s `daemon=` does.

## Nothing is printed

`println!`, `eprintln!`, `print!`, `eprint!`, and `dbg!` do not appear in this codebase. Everything `mercury` says goes through `tracing`, so the log file is the whole record of a run rather than the part that did not go to a terminal. The terminal is a `tracing_subscriber` layer exactly as the file is.

A client verb's level is its audience:

- `info!` is the verb's answer. It reaches stdout, and there is one per invocation.
- `warn!` and `error!` are problems the user has to see. They reach stderr.
- `debug!` is what the verb did along the way. Only the file keeps it.

The daemon is different: its terminal is its log in full, filtered by `--log-level`.

`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`.

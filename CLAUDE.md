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

## Tests

The standard for the model is exhaustive: every key in every reachable state, asserting exactly what dispatch produces. The model is a pure function of state and event, so the full table is checkable and doubles as documentation of the keymap. Not all of it exists yet; new bindings should extend toward it rather than test only the happy path.

## Logs

mercury writes its tracing output to `~/Library/Logs/mercury/mercury.log`, always, appending across runs. Read that file to debug a run.

The file always records down to `debug`, whatever the terminal is set to, so a run is always reconstructable afterwards. It holds one record per dispatched event, carrying the event, the effects it produced, and the resulting state on a single line, plus each key emitted, each app foregrounded, and the raw frontmost-app changes `freddie_app_nav` observed.

`LOG_LEVEL` sets what the terminal shows and nothing else, defaulting to `info`. So `LOG_LEVEL=error cargo run -p mercury` gives a quiet terminal and a full log file. Watch it live from another pane: `tail -f ~/Library/Logs/mercury/mercury.log`.

## Coding standards

- Maintainability is the most important standard. And that specifically means one thing: make impossible states unrepresentable and use the correct underlying representation or building blocks. If a field is not used when a boolean is true/false, use an option, for example.
- If we have to do extra refactoring work to maintain the above, we should do the extra work. If we need to refactor large parts of freddie in order to have the right building blocks, then we will do that.
- If we need a more performant, but less idiomatic impl, then create a newtype/struct/enum that encapsulates the ugly complexity but exposes an idiomatic API.
- If a comment provides no more information than one would get by reading the code, do not include the comment.
- A comment should not describe what wasn't done, ESPECIALLY if "we didn't do x" is more indicative of the fact that we either previously discussed doing X or in a previous iteration of a planning doc, you suggested doing X.
- In JavaScript, a discriminated union takes exactly one form: `{ kind: "Type.Variant", value: T }`. The tag is always `kind`, its value is the dotted `Type.Variant` name, and the payload is always the single `value` field (never inline fields, never a bare variant name). Every variant that shares a `Type` prefix belongs to the same union, so `Type.` is how you read off which union a value is in.

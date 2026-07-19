# a protocol that names the state it is talking about

Ideas, not a plan. The external protocol is fire-and-forget in both directions, which is enough for what it carries today and runs out the moment a client wants to ask a question whose answer depends on when it was asked.

The two problems here are the ones matklad names in [LSP could have been better](https://matklad.github.io/2023/10/12/lsp-could-have-been-better.html), under "Causality Casualty" and "Remote Procedural State Synchronization". The third, forking, is past what that post asks for and is the one that would actually change how freddie is built.

## What is wrong now

`external-events.md` takes events and `external-effects.md` sends commands and takes replies. A reply is matched to its command by a monotonic id and nothing else. Nothing in either direction says which state anything happened against.

So a client cannot tell the difference between these:

- It asked for the front tab's site, mercury answered `claude.ai`, and that is still true.
- It asked for the front tab's site, mercury answered `claude.ai`, and by the time the answer arrived the front tab was something else.

The id says the answer belongs to the question. It does not say the answer is still true. Today that is fine, because nothing acts on a reply. It stops being fine the moment a client does something conditional on one.

matklad's version of this is sharper, because in LSP the two directions cross: a client sends `didChangeTextDocument` and then receives a `workspace/applyEdit` computed before that change landed. The edit is stale and nothing in the protocol says so. LSP's answer is document version numbers, which he calls best-effort and which fail exactly where you would expect: a rename computed against one version, applied to a document that has since grown another usage, is "valid" by version number and wrong in fact.

## Causality: every message names a state

Give each state mercury reaches an id, and put it on everything:

```jsonc
// mercury -> client
{ "kind": "OutgoingEffect.Command", "value": { "id": 42, "state": "s17", "command": { … } } }

// client -> mercury
{ "kind": "IncomingEvent.Reply", "value": { "id": 42, "state": "s17", "result": { … } } }
```

The reply naming `s17` is the client saying "this was computed against what you told me `s17` was". mercury, at `s19` by then, can see that and decide: apply it anyway, refuse it, or recompute. What it cannot do today is know there was a question.

The dispatch loop already has the shape for this. `state.handle(event)` is one state to the next, one event at a time, so an id per dispatch is a counter. What it does not have is any way to answer a question about `s17` once it is at `s19`, which is the next section.

## Level-triggered, not edge-triggered

matklad's second criticism is that LSP is an RPC protocol where it should be a state synchronization protocol: "The client and the server need to agree what something _is_, deciding the course of action is secondary." The Dart analysis server's model is his example, where a client subscribes to a set of files and the server pushes their state as it changes, rather than the client asking again each time.

Mercury's version of this is that a client should be able to say "tell me the front tab's site whenever it changes" and stop asking. What it subscribes to is a projection of the state, and the two things kept in sync are the subscription set and the values.

That happens to be what the site layer already does internally. `site_data` is not a question anyone asks; it is recomputed from the root on every dispatch, so it cannot be stale. The subscription model is that same idea offered across the wire, and the argument for it is the same argument: a value derived on demand cannot disagree with what it was derived from, and a value fetched once can.

## Forking, which is the interesting one

The thing the post does not ask for, and the thing that would change freddie:

```
from s0, apply X  ->  s1
from s1, apply Y  ->  s2
from s2, what are the errors?
from s0, apply Z  ->  s3
```

Every message names the state it starts from, so the states form a DAG rather than a line, and `s1` staying alive after `s2` exists is what makes the last line possible. A client can explore two futures from one past, ask questions about either, and never coordinate with anything else asking questions of its own.

For a language server the case is obvious: speculative edits, "what would this refactoring do", answering a stale request against the state it was asked about instead of failing it.

For mercury the honest case is thinner, and worth writing down rather than pretending otherwise:

- Tests. `transitions.rs` builds a state, dispatches, and asserts. Forking is exactly "from this state, try each of these twenty keys", which is the exhaustive-keymap table `CLAUDE.md` asks for, expressed directly rather than by rebuilding the state twenty times.
- A client that wants to know what a key would do without doing it. An overlay that shows the outcome of each binding rather than its name.
- Replay. A log of `(state id, event)` reconstructs any state, which the log already almost is: it records the event, the effects, and the resulting state on one line.

## What it would cost

This is where the idea meets what freddie is. Keeping `s1` alive after `s2` exists means a state that is not mutated in place, and laserbeam's whole shape is a typed mutable path into a tree. `PathMut` mutates through, handlers take `&mut`, and dispatch writes into the state it was handed.

So forking means one of:

- Cloning the whole tree per state. Simple, correct, and the cost is proportional to the tree rather than to the change. The tree is small, so this is not obviously wrong; it is what the tests would do anyway.
- Persistent data structures with structural sharing, which is the real answer for a language server and a rewrite of laserbeam's core for mercury.
- Keeping only the events. A state id is an event log offset, and asking about `s17` means replaying from the root. Cheap to store, and the cost is the replay, which for a keyboard remapper is microseconds and for a language server is not.

The third one is the one worth thinking about first, because it needs nothing from laserbeam: dispatch is already a pure function of state and event, the log already records both, and "fork" becomes "replay these events instead of those".

## What this is not for

The Chrome extension. It sends a URL and takes commands about the page in front of you, and every answer it could give is about right now. Nothing there wants to ask about a state mercury has left.

That is the reason this is a separate doc from `external-effects.md`, which should ship as it is.

## Open

- Whether state ids are a counter, an event-log offset, or a hash of the state. The offset is the one that makes replay free.
- Whether a client may name a state mercury has forgotten, and what it gets back. LSP's answer to the equivalent is an error and a resync; here the equivalent is replay, which never fails but can be slow.
- Whether subscriptions and forking are one feature or two. A subscription is a standing question about the newest state, and a fork is a one-off question about an old one; they meet in the middle if a subscription can be pinned to a state.
- Whether any of this earns its keep before there is a second client. The CLI and the test harness are the ones that would use it, and neither exists.

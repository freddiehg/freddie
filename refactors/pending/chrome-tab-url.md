# track Chrome's selected tab URL

Not built. This is the concrete case that makes `ChromeApp` carry data.

Today `ChromeApp {}` is a unit struct: mercury tracks nothing per app, so the derived
app level has no data (see the punt in `derived-levels-plan.md`, step 8). Tracking the
front tab's URL is the first real per-app state, so `ChromeApp` becomes
`ChromeApp { url: String }` (or a richer tab struct), and the derived child fn
`app_data` clones it into the level a handler sees.

## What is missing: a source

`freddie_app_nav` reports which app is frontmost. Nothing reports Chrome's active tab.
That needs a new source, the browser-tab analog of the app watcher:

- Read the current URL of Chrome's active tab, and
- watch for it changing (tab switch, navigation).

On macOS this is Apple Events / AppleScript to Chrome (`tell application "Google Chrome"
to get URL of active tab of front window`), or the accessibility tree. Either way it is
a poll-or-subscribe source that emits a `TabChanged { url }` event, the same shape as the
foreground watcher emitting `foreground(app)`.

## How it flows through the model

- A new event variant, `MercuryEvent::Tab(TabEvent { url })`, and a trigger for it, the
  same way `Foregrounded` is a source. A handler on the root (or on `ChromeApp`) writes
  the URL into state.
- Where the URL lives: on `ChromeApp` in the tree, so `app_data` reads it and the in-app
  Chrome level's handlers can use it (open-in-editor, copy-url, etc.).

## Open

- Whether the tab source polls or subscribes, and its cost. A poll every N ms is simple;
  Apple Events subscription is lower-latency but more fragile.
- Permissions: reading another app's tab needs automation entitlement, which the user
  grants once.
- Whether this generalizes to other browsers (Safari, Arc) behind one `TabEvent`, the way
  `App::from_bundle_id` collapses apps.

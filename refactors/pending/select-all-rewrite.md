# select all and rewrite

A binding that takes everything in the thing you are editing, hands it to a rewriter, and puts the result back. effects-and-events.md sketches the general, any-app version (`cmd-a`, `cmd-x`, clipboard, paste) and the four ways it goes wrong. This doc adds what that sketch could not have: on a site the extension can script, the whole clipboard-and-focus circus disappears, so site layers get the good version and the generic version waits.

## The site version

On a site with a known editor (claude.ai's prompt box first), the extension reads and writes the editor's contents directly through `chrome.scripting.executeScript`, addressed at the tab by id. No clipboard, no `cmd-a`, no focus requirement: the tab does not even have to be frontmost, and the user can keep typing elsewhere while the rewrite runs.

The flow is a state machine across the socket, riding the command chain (external-effects.md, scoped-commands.md, extension-commands.md):

1. The binding emits a site-scoped command, `ClaudeAiCommand::ReadEditor`, addressed at the tab in state.
2. The extension runs a site-specific routine in the tab (each site names its editor; claude.ai's is a ProseMirror contenteditable, and the selector lives in `src/sites/claudeAi.ts` next to the routine) and reports the text back as an event: `IncomingEvent::Editor(EditorMessage { tab, text })`.
3. The handler stores the original and enters a rewriting state; a detached thread runs the rewriter; completion comes back as an event carrying the new text.
4. That event's handler emits `ClaudeAiCommand::SetEditor(SetEditor { text })`, and the extension replaces the editor's contents, guarded the way extension-commands.md already guards navigation: if the tab has left the site, the command is dropped and the event that says so puts the original text nowhere, because the original is still safely in the model.

The failure story that dominates the generic version mostly evaporates: the text is never only in the clipboard (the model holds it from step 2 on), nothing is cut (the editor keeps its contents until `SetEditor`), and a failed or timed-out rewrite ends the state machine with the editor untouched.

```rust
/// Lives on the rewriting site's level while a rewrite is in flight.
#[derive(Debug)]
pub struct Rewrite {
    pub tab: TabId,
    /// What the editor held when this started; what SetEditor restores on a failed rewrite
    /// if the editor was already cleared, and what makes a retry possible.
    pub original: String,
}
```

## The rewriter

The rewriter is a subprocess: `claude -p <prompt>` with the text on stdin, output captured, completion delivered as an event by the thread that ran it. It takes seconds, which is why it is a state and never an effect's body. The prompt is data on the binding (`handlers-as-values.md` is the shape), so "tighten this", "fix grammar", and "translate" are the same machinery with different strings, and the first binding ships with exactly one prompt rather than a prompt-picking layer.

## The generic version

Outside the browser the extension cannot help, and the clipboard version from effects-and-events.md applies, with all four hazards (clipboard as the only copy, clobbering the user's clipboard, focus moving before paste, keys arriving mid-rewrite). It is explicitly second; building the site version first means the state machine, the rewriter subprocess, and the completion event all exist before the hard delivery problem is opened.

## Open questions

- The rewriter invocation: `claude -p` versus the API directly, and which model. This is a taste-and-latency call.
- The first prompt's wording, and which key on the site layer it takes.
- Whether a finished rewrite ends in typing (you review and continue in the editor: likely) or stays in the site layer.

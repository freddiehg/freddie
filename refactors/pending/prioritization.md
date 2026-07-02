# prioritization

The runner processes events FIFO, and the framework hands effects to the handler in that order. A later runner will let some things jump ahead: the motivating case is typing, which should be handled promptly and in order relative to itself, ahead of lower-priority work.

## Two levers

Priority can sit in two different places, and they are not the same:

- Event queue. Dispatch high-priority events before others. This reorders dispatch itself, so it changes which state transitions fire when. A bigger hammer, and easy to get wrong.
- Effect handling. Dispatch stays FIFO; the handler performs some effects (typing) ahead of others. This is what the typing example points at: the state machine is untouched, only the outside-world work is reordered.

Effect-level is the likelier home; event-level is available if a case needs it.

## Generic queue

`SimpleRunner` holds a `VecDeque` and is FIFO. To make priority a drop-in, the runner could be generic over a small queue trait:

```rust
trait Queue<T> {
    fn push(&mut self, item: T);
    fn pop(&mut self) -> Option<T>;
}
```

`VecDeque` implements it (`push_back`/`pop_front`); a priority queue implements it over a heap.

Two catches with a plain `BinaryHeap`:

- It needs `T: Ord`, so the item must carry its priority. Priority is usually per-source or per-binding, not intrinsic to the event, so the item is really `(Priority, Event)`, not `Event`.
- `BinaryHeap` is not stable: equal-priority items pop in arbitrary order. Typing needs FIFO among equal priority, so the item needs an insertion counter, `(Priority, Seq, Event)`, with the runner bumping `Seq` on each push.

## For now

`SimpleRunner` stays on `VecDeque`. The `Queue` trait is a few lines to add when the fancier runner needs it, and the priority queue should be the stable `(Priority, Seq, _)` form. Whether it keys events or effects is decided then.

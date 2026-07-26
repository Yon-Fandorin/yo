# Rendering parity fixture

This fixture pins two projections of one completed 6×2 `Surface`.

- `expected.ansi.txt` shows terminal bytes with ESC and blank bytes encoded as
  the literals `\x1b` and `\x20`, so the golden remains readable and diffable.
- `expected.html` is the canonical flow-content fragment.
- `expected.css` is the document-level CSS required by the fragment.

The shared case covers a one-cell ASCII grapheme, two-cell Korean and emoji
graphemes, continuation ownership, an explicitly styled blank, HTML escaping,
RGB and indexed colors, and every resolved text attribute.

Run it with:

```text
cargo test -p yo-tui --test rendering_parity
```

These deterministic goldens detect adapter drift. They do not replace the
separate real-PTY and browser environment matrix.

# Super Pong spec mirror

An executable mirror of [`docs/SPEC-superpong.md`](../../docs/SPEC-superpong.md), written in Python from the spec's text and pseudo-code (sections 5-11 and 14.3), independently of the Rust core. It produces the golden data in [`game/superpong-core/testdata/`](../../game/superpong-core/testdata/), so the core's tests check the Rust code against a second reading of the spec, not against itself.

Nothing here is built or shipped; it is the provenance of those fixtures.

| File | Contents |
|---|---|
| `core_ref.py` | The console: knobs, objects, ball physics, scoring, the light show, Start and Game Select, and the renderer in the `Frame::to_ascii` format (`.` background, `:` border, `#` foreground). |
| `gen_fixtures.py` | Writes the six golden frames and `trace.txt` (three 40,000-tick scripted runs: events, status every 500 ticks, an FNV-1a hash of the frame every 2,000 ticks). The script is the same xorshift player as `Script` in `src/tests.rs`. |

## Running

Any Python 3 (3.8 or later); standard library only. From the repository root:

```
python3 -B tools/superpong-mirror/gen_fixtures.py game/superpong-core/testdata
cargo test -p superpong-core
```

It runs in about a second and prints `ok`. Then review the diff of `testdata/`.

## When the spec changes

1. Change the spec.
2. Change `core_ref.py` from the spec text, without looking at the Rust change.
3. Regenerate the fixtures, change the core, and run its tests. The Rust core and the mirror must agree byte for byte on every fixture and on the trace.

Never regenerate the fixtures from the Rust core: the tests would then pass by definition.

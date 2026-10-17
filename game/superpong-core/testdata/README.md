# superpong-core test fixtures

These files are expected outputs for `src/tests.rs`. They were not written by this crate. An independent executable mirror of `docs/SPEC-superpong.md` (sections 5-11 and 14.3, in Python) produced them: [`tools/superpong-mirror/`](../../../tools/superpong-mirror/), run as `python3 -B tools/superpong-mirror/gen_fixtures.py game/superpong-core/testdata` from the repository root. So a test that compares against them checks the Rust code against a second reading of the spec, not against itself.

| File | Test | Contents |
|---|---|---|
| `power_on_catch.txt` | `golden_power_on_frame` | Frame 0 after power-on, Catch, knobs centred (`Frame::to_ascii`). |
| `attract_catch_t376.txt` | `golden_light_show_frame` | The light show 376 ticks after power-on: h0 12, V wrap on line 173. |
| `superpong_rally.txt` | `golden_super_pong_rally_frame` | Super Pong in Rally: tops 40 / 150, ball at (61, 101). |
| `solitaire_rally.txt` | `golden_solitaire_rally_frame` | Solitaire in Rally: paddle top 120, wall top 69, ball at (100, 50). |
| `pong_waiting_10_5.txt` | `golden_pong_waiting_frame_10_5` | Pong in Waiting with the scores 10-5. |
| `game_over_15_4.txt` | `golden_game_over_frame` | The frame of the 15th point, Pong 15-4, which is drawn as the light show. |
| `trace.txt` | `differential_trace_against_the_spec_mirror` | Three 40,000-tick scripted runs (Super Pong, Catch, Pong) driven by the xorshift script in `tests.rs` (`Script`). Each line is either an event (`tick Event`), a status line every 500 ticks, or an FNV-1a hash of the rendered frame every 2,000 ticks. |

When the spec changes, update the mirror from the spec, regenerate these files from it and review the diff. Regenerating them from this crate would make the tests pass by definition.

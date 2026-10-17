"""Writes the superpong-core test fixtures from the spec mirror (core_ref.py).

Usage: gen_fixtures.py <out_dir>

Golden frames (Frame::to_ascii format) and a differential trace of a long
scripted run. The Rust tests replay the same script and compare.
"""
import sys, os
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from core_ref import *

OUT = sys.argv[1]
os.makedirs(OUT, exist_ok=True)
M = (1 << 64) - 1

def kt(t): return (min(max(t, 0), 209) + 0.5) / 210.0

def setball(c, x, row, dirx, vy, visible=True, volleys=0):
    c.x, c.row, c.dirx, c.vy, c.volleys = x, row, dirx, vy, volleys
    c.phase = RALLY if visible else WAITING

def frame_text(c):
    return ascii(c.render()) + "\n"

def write(name, text):
    with open(os.path.join(OUT, name), 'w') as f:
        f.write(text)

# --- golden frames -------------------------------------------------------
# 1. Power-on frame 0, Catch, knobs centred.
c = Console(CATCH)
write('power_on_catch.txt', frame_text(c))

# 2. Light show 376 ticks after power-on (img/ls_compare_power_on_t470.00.png: h0 12, wrap line 173).
c = Console(CATCH)
for _ in range(376): c.tick()
assert (c.h0, c.last_wrap) == (12, 173), (c.h0, c.last_wrap)
write('attract_catch_t376.txt', frame_text(c))

# 3. Super Pong rally: tops 40 / 150, ball visible at (61, 101).
c = Console(SUPERPONG)
c.tick(kt(40), kt(150))
setball(c, 61, 101, 1, 1)
write('superpong_rally.txt', frame_text(c))

# 4. Pong waiting with scores 10-5 (10 right exits, then 5 left exits).
c = Console(PONG)
for _ in range(10):
    setball(c, 174, 100, 1, 0); c.tick(kt(20), kt(180))
for _ in range(5):
    setball(c, 0, 100, -1, 0); c.tick(kt(20), kt(180))
assert c.scores == [10, 5] and c.phase == WAITING, (c.scores, c.phase)
write('pong_waiting_10_5.txt', frame_text(c))

# 5. Solitaire rally: paddle top 120, wall top 69, ball at (100, 50).
c = Console(SOLITAIRE)
c.tick(kt(120), kt(69))
setball(c, 100, 50, -1, -2)
write('solitaire_rally.txt', frame_text(c))

# 6. Game over at 15-4 (Pong): the frame of the 15th point is light show.
c = Console(PONG)
for _ in range(14):
    setball(c, 174, 100, 1, 0); c.tick(1.0, 1.0)
for _ in range(4):
    setball(c, 0, 100, -1, 0); c.tick(1.0, 1.0)
assert c.scores == [14, 4]
setball(c, 172, 84, 1, 1)
for _ in range(3): ev = c.tick(1.0, 1.0)
assert ev == [('Point',)] and c.phase == ATTRACT and c.scores == [15, 4], (ev, c.phase, c.scores)
write('game_over_15_4.txt', frame_text(c))

# --- differential trace --------------------------------------------------
def fnv64(s):
    h = 0xcbf29ce484222325
    for b in s.encode():
        h ^= b
        h = (h * 0x100000001b3) & M
    return h

class Rng:
    def __init__(s, seed): s.s = seed
    def next(s):
        x = s.s
        x ^= (x << 13) & M
        x ^= x >> 7
        x ^= (x << 17) & M
        s.s = x
        return x

def ev_name(e):
    return f"Bounce({e[1]})" if e[0] == 'Bounce' else e[0]

def trace(kind, seed, ticks):
    c = Console(kind)
    rng = Rng(seed)
    off_l = off_r = 0
    tl = tr = 105
    out = []
    for t in range(1, ticks + 1):
        r1, r2, r3 = rng.next(), rng.next(), rng.next()
        if r1 % 50 == 0: off_l = (r2 % 41) - 20
        if (r1 >> 16) % 50 == 0: off_r = ((r2 >> 16) % 41) - 20
        start = (c.phase == ATTRACT and (r3 >> 16) % 300 == 0) or r3 % 25000 == 0
        select = ((r3 >> 48) % 4) + 1 if (r3 >> 32) % 2500 == 0 else None
        if c.phase == RALLY:
            tl = c.row - 6 + off_l
            tr = c.row - 6 + off_r
        ev = c.tick(kt(tl), kt(tr), start, select)
        for e in ev: out.append(f"{t} {ev_name(e)}")
        if t % 500 == 0:
            if c.phase == ATTRACT: ball = "-"
            else: ball = f"{c.x} {c.row} {c.dirx} {c.vy}"
            out.append(f"{t} S {c.phase} {c.kind} {c.scores[0]} {c.scores[1]} {c.volleys} {c.lt} {c.rt} {ball}")
        if t % 2000 == 0:
            out.append(f"{t} F {fnv64(ascii(c.render()) + chr(10)):016x}")
    return out

lines = ["# Differential trace from the spec mirror; see tests.rs `differential_trace`."]
for kind, seed in ((SUPERPONG, 0x9E3779B97F4A7C15), (CATCH, 0x0123456789ABCDEF), (PONG, 0x5DEECE66D2545F49)):
    lines.append(f"run {kind} {seed:016x} 40000")
    lines += trace(kind, seed, 40000)
write('trace.txt', "\n".join(lines) + "\n")
print("ok")

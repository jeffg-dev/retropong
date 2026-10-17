"""Executable mirror of docs/SPEC-superpong.md (sections 5-11 and 14.3), written from
the spec, independently of the Rust core. Not shipped; it validates the spec's
pseudo-code and produces the golden data in game/superpong-core/testdata/ (see
gen_fixtures.py and README.md)."""
import math
CATCH, SOLITAIRE, SUPERPONG, PONG = 1, 2, 3, 4
ATTRACT, WAITING, RALLY = 'Attract', 'Waiting', 'Rally'
SEG_VY = [-3, -2, -1, 0, 0, 1, 2, 3]
HIDDEN_X_MIN, HIDDEN_WRAP_RIGHT, HIDDEN_WRAP_LEFT = -25, 228, 227   # spec §6.9: direction-dependent
PLAY_LINES, ROW_MIN = 263, -40
TOP_MIN = -6    # spec §6.3: the top border test covers rows -6..=15 (light-show top zone, lines 258..=261 and 0..=17)
SERVE_X_RIGHT, SERVE_X_LEFT = 87, 90
START_X = 112
LS_H, LS_LINES = 228, 262
LS_DEAD = (172, 223)
POWER_ON_H0, POWER_ON_WRAP = 92, 177    # spec §10.4: the video's first console frame (463.730 s)
GAME_OVER_DX, GAME_OVER_DROW = 0, 8     # spec §10.6: h0 = x - 0, wrap line = row + 8
ATTRACT_TICK_H0 = 82
GLYPHS = {
 0:["####","#..#","#..#","#..#","#..#","#..#","#..#","####"],
 1:["...#"]*8,
 2:["####","...#","...#","####","#...","#...","#...","####"],
 3:["####","...#","...#","####","...#","...#","...#","####"],
 4:["#..#","#..#","#..#","####","...#","...#","...#","...#"],
 5:["####","#...","#...","####","...#","...#","...#","####"],
 6:["####","#...","#...","####","#..#","#..#","#..#","####"],
 7:["####","...#","...#","...#","...#","...#","...#","...#"],
 8:["####","#..#","#..#","####","#..#","#..#","#..#","####"],
 9:["####","#..#","#..#","####","...#","...#","...#","####"],
}
def knob_to_top(k):
    if k != k: k = 0.5
    k = min(1.0, max(0.0, k))
    return min(209, math.floor(k * 210))
def tier(v): return 1 if v <= 3 else (2 if v <= 7 else 3)

def objects(kind, lt, rt):
    P = lambda x, top, ref: ('Paddle', x, ref, ('paddle', top))
    if kind == PONG: return [P(16, lt, -1), P(160, rt, 1)]
    if kind == SUPERPONG: return [P(16, lt, -1), P(80, lt, -1), P(96, rt, 1), P(160, rt, 1)]
    if kind == SOLITAIRE: return [P(16, lt, -1), ('Wall', 160, 1, ('sol', rt))]
    if kind == CATCH: return [('Wall', 16, -1, ('catch', lt)), ('Wall', 160, 1, ('catch', rt))]
def rows_overlap(shape, R):
    t, top = shape
    if t == 'paddle': return R < top + 15 and R + 4 > top
    if t == 'sol': return R + 4 > top and R < 224
    if t == 'catch':
        return any((r < top or r >= top + 15) for r in range(max(R, 0), min(R + 4, 224)))
def new_vy(shape, R):
    t, top = shape
    if t == 'paddle': return SEG_VY[min(7, max(0, (R - top) // 2))]
    if t == 'catch': return -3 if R < top else 3
    if t == 'sol': return -3 if R < top else SEG_VY[((R - top) % 16) // 2]

class Console:
    def __init__(s, kind=SUPERPONG):
        s.kind = kind; s.phase = ATTRACT; s.scores = [0, 0]
        s.lt = s.rt = knob_to_top(0.5)
        s.x, s.row, s.dirx, s.vy, s.volleys = 0, 100, 1, 1, 0
        # light show power-on state: h0 92, wrap at line 177, vy +1
        s.h0 = POWER_ON_H0; s.lvy = 1; s.vc = (LS_LINES + 1 - 1 - POWER_ON_WRAP) % (LS_LINES + 1); s.last_wrap = 0
        s.rows_lit = [False] * 224
        s.run_lines(); s.frame = 0
    def frame_hz(s): return 60.19 if s.phase == ATTRACT else 59.94
    # ---- tick ----
    def tick(s, left=0.5, right=0.5, start=False, select=None):
        ev = []
        s.lt, s.rt = knob_to_top(left), knob_to_top(right)
        if select is not None: s.kind = select
        if start: s.start_game()
        if s.phase == ATTRACT: s.attract_frame(ev)
        else: s.play_frame(ev)
        s.frame += 1
        return ev
    def start_game(s):
        if s.phase == ATTRACT:
            s.row = (s.last_wrap - 2 - ROW_MIN) % PLAY_LINES + ROW_MIN
            s.vy = s.lvy
        s.scores = [0, 0]; s.x = START_X; s.dirx = 1; s.volleys = 0; s.phase = WAITING
    def play_frame(s, ev):
        was_hidden = s.phase == WAITING
        sp = 1 if was_hidden else tier(s.volleys)
        s.x += s.dirx * sp; s.row += s.vy
        if was_hidden:
            wrap = HIDDEN_WRAP_RIGHT if s.dirx > 0 else HIDDEN_WRAP_LEFT
            s.x = (s.x - HIDDEN_X_MIN) % wrap + HIDDEN_X_MIN
        s.row = (s.row - ROW_MIN) % PLAY_LINES + ROW_MIN
        if 0 <= s.x <= 175 and ((s.vy < 0 and TOP_MIN <= s.row <= 15) or (s.vy > 0 and s.row + 3 >= 208)):
            s.vy = -s.vy
            if not was_hidden: ev.append(('Bounce', 'Border'))
        if was_hidden:
            if (s.dirx > 0 and s.x == SERVE_X_RIGHT) or (s.dirx < 0 and s.x == SERVE_X_LEFT):
                s.phase = RALLY; ev.append(('Serve',))
            return
        for kind, ox, ref, shape in objects(s.kind, s.lt, s.rt):
            if s.dirx == ref and s.x < ox + 4 and s.x + 3 > ox and rows_overlap(shape, s.row):
                s.dirx = -s.dirx; s.volleys += 1; s.vy = new_vy(shape, s.row)
                ev.append(('Bounce', kind)); break
        if (s.dirx > 0 and s.x + 1 >= 176) or (s.dirx < 0 and s.x + 1 <= 0):
            exit_side = 1 if s.dirx > 0 else 0          # 0 = left, 1 = right
            scorer = exit_side if s.kind == CATCH else 1 - exit_side
            s.scores[scorer] += 1; s.volleys = 0; ev.append(('Point',))
            if s.scores[scorer] >= 15: s.enter_attract_from_ball()
            else: s.phase = WAITING
    # ---- attract ----
    def enter_attract_from_ball(s):
        s.phase = ATTRACT
        s.h0 = (s.x - GAME_OVER_DX) % LS_H
        s.lvy = s.vy
        v = s.row + GAME_OVER_DROW
        p = LS_LINES + s.lvy
        s.vc = (p - 1 - v) % p
        s.run_lines()
    def attract_frame(s, ev):
        s.h0 = (s.h0 + 1) % LS_H
        if s.h0 == ATTRACT_TICK_H0: ev.append(('AttractTick',))
        s.run_lines()
    def run_lines(s):
        s.wraps = []
        s.last_wrap -= LS_LINES
        for l in range(LS_LINES):
            s.vc += 1
            if s.vc >= LS_LINES + s.lvy:
                s.vc = 0; s.last_wrap = l; s.wraps.append(l)
                live = not (LS_DEAD[0] <= s.h0 <= LS_DEAD[1])
                if live and s.lvy < 0 and (l <= 17 or l >= 258): s.lvy = -s.lvy
                elif live and s.lvy > 0 and 207 <= l <= 225: s.lvy = -s.lvy
            if l < 224: s.rows_lit[l] = s.vc >= 256 or (s.vc & 16) == 0
    # ---- render ----
    def render(s):
        W, H = 187, 240
        f = [[0] * W for _ in range(H)]
        def put(x, r, v):
            if -6 <= x <= 180 and -8 <= r <= 231: f[r + 8][x + 6] = max(f[r + 8][x + 6], v)
        def fg(x, r):
            if 0 <= r < 224: put(x, r, 2)
        for r in list(range(0, 16)) + list(range(208, 224)):
            for x in range(176): put(x, r, 1)
        for r in range(224):
            if r % 16 < 8:
                for x in range(88, 92): fg(x, r)
        for kind, ox, ref, (t, top) in objects(s.kind, s.lt, s.rt):
            for r in range(224):
                on = (t == 'paddle' and top <= r < top + 15) or (t == 'sol' and r >= top) or (t == 'catch' and not (top <= r < top + 15))
                if on:
                    for x in range(ox, ox + 4): fg(x, r)
        if s.phase == RALLY:
            for r in range(s.row, s.row + 4):
                for x in range(s.x, s.x + 2): fg(x, r)
        if s.phase != RALLY:
            for val, ux, tx in ((s.scores[0], 40, 24), (s.scores[1], 136, 120)):
                if val >= 10:
                    for r in range(32, 96):
                        for x in range(tx, tx + 4): fg(x, r)
                g = GLYPHS[val % 10]
                for gr in range(8):
                    for gc in range(4):
                        if g[gr][gc] == '#':
                            for r in range(32 + 8 * gr, 40 + 8 * gr):
                                for x in range(ux + 4 * gc, ux + 4 * gc + 4): fg(x, r)
        if s.phase == ATTRACT:
            for r in range(224):
                if 32 <= r <= 95 or not s.rows_lit[r]: continue
                for x in range(176):
                    Hc = (x - s.h0) % LS_H
                    if Hc < 224 and (Hc & 8) == 0: fg(x, r)
        return f
def ascii(f):
    return "\n".join("".join(".:#"[v] for v in row) for row in f)

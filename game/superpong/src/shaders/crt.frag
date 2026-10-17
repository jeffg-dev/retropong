#version 100
// Classic video (SPEC §13.1): the picture as a 1970s colour TV showed it.
//
// Based on Timothy Lottes' "PUBLIC DOMAIN CRT STYLED SCAN-LINE SHADER"
// (public domain; libretro glsl-shaders, crt/shaders/crt-lottes.glsl): the
// barrel warp, Gaussian scanlines and horizontal filter worked in linear
// light, the phosphor mask and the small bloom. Changed for a TV fed
// composite video:
// - the horizontal filter runs on the video signal (YIQ, before gamma),
//   narrow for luma and wide for chroma, so colour bleeds at object edges
//   while the one-clock stripe texture stays visible;
// - the beam widens with brightness and is normalised so every line keeps
//   its average level (the idea crt-geom and crt-pi use too);
// - a dark tube face with rounded corners around the curved picture, and a
//   vignette;
// - scanlines and mask fade out when lines are under 3 device pixels tall,
//   where they would only make moire.

#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif

varying vec2 uv;

// The picture: 8-bit sRGB, four sub-columns per clock, one row per line.
uniform sampler2D Texture;
// Its size in texels (748 x 240).
uniform vec2 SourceSize;
// The picture rectangle on screen, in device pixels.
uniform vec2 OutputSize;

// Barrel warp, as in crt-lottes (whose defaults are 0.031 and 0.041).
const vec2 WARP = vec2(0.024, 0.032);
// Corner radius of the tube face, in picture heights.
const float CORNER = 0.035;
// Video gamma: the picture's levels are decoded with it and re-encoded.
const float GAMMA = 2.2;
// Luma and chroma filter widths (standard deviations), in texels; a clock
// is 4 texels. Luma passes the one-clock stripes; chroma spreads a clock.
const float LUMA_SIGMA = 0.6;
const float CHROMA_SIGMA = 3.5;
// Beam profile across a line (standard deviation, in lines) for a black
// and a full-white line.
const float BEAM_DARK = 0.25;
const float BEAM_BRIGHT = 0.40;
// Phosphor mask: level of the two unlit stripes of each RGB triad.
const float MASK_DARK = 0.7;
// Glow: radius (standard deviation, in line heights) and amount added.
const float GLOW_SIGMA = 1.3;
const float GLOW = 0.10;
// Width of a line height in texels: a clock is 1.711 lines wide (SPEC §13.1).
const float TEXELS_PER_LINE = 4.0 / 1.711;
// Vignette exponent: higher darkens the edges more.
const float VIGNETTE = 0.12;

vec3 fetch(float tx, float ty) {
    tx = clamp(tx, 0.0, SourceSize.x - 1.0);
    return texture2D(Texture, (vec2(tx, ty) + 0.5) / SourceSize).rgb;
}

// FCC YIQ, the colour space of the composite filter (SPEC §13.1).
float luma(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

vec2 chroma(vec3 c) {
    return vec2(dot(c, vec3(0.596, -0.274, -0.322)), dot(c, vec3(0.211, -0.523, 0.312)));
}

vec3 from_yiq(float y, vec2 iq) {
    return vec3(
        y + 0.956 * iq.x + 0.621 * iq.y,
        y - 0.272 * iq.x - 0.647 * iq.y,
        y - 1.106 * iq.x + 1.703 * iq.y);
}

vec3 to_linear(vec3 c) {
    return pow(clamp(c, 0.0, 1.0), vec3(GAMMA));
}

float gauss(float d, float sigma) {
    return exp(-0.5 * d * d / (sigma * sigma));
}

// Line `ty` at texel position `sx` after the video filters, in linear
// light. Lines outside the picture are black.
vec3 scan_line(float ty, float sx) {
    if (ty < 0.0 || ty > SourceSize.y - 1.0) {
        return vec3(0.0);
    }
    float tx = floor(sx);
    float y = 0.0;
    float wy = 0.0;
    for (int k = -2; k <= 2; k++) {
        float t = tx + float(k);
        float w = gauss(t + 0.5 - sx, LUMA_SIGMA);
        y += w * luma(fetch(t, ty));
        wy += w;
    }
    vec2 iq = vec2(0.0);
    float wc = 0.0;
    for (int k = -5; k <= 5; k++) {
        float t = tx + 2.0 * float(k);
        float w = gauss(t + 0.5 - sx, CHROMA_SIGMA);
        iq += w * chroma(fetch(t, ty));
        wc += w;
    }
    return to_linear(from_yiq(y / wy, iq / wc));
}

// The light a line of colour `c` gives `d` lines from its centre: a
// Gaussian that widens with brightness, scaled so its integral is `c`.
vec3 beam(vec3 c, float d) {
    vec3 sigma = mix(vec3(BEAM_DARK), vec3(BEAM_BRIGHT), sqrt(c));
    vec3 x = d / sigma;
    return c * exp(-0.5 * x * x) / (sigma * 2.5066283);
}

// A wide, soft blur of the picture around texel position `s`, in linear
// light, round in screen space.
vec3 glow(vec2 s) {
    vec2 t0 = floor(s);
    vec3 sum = vec3(0.0);
    float wsum = 0.0;
    for (int j = -2; j <= 2; j++) {
        for (int i = -2; i <= 2; i++) {
            vec2 t = t0 + vec2(3.0 * float(i), float(j));
            vec2 d = (t + 0.5 - s) / vec2(TEXELS_PER_LINE, 1.0);
            float w = exp(-0.5 * dot(d, d) / (GLOW_SIGMA * GLOW_SIGMA));
            sum += w * to_linear(fetch(t.x, clamp(t.y, 0.0, SourceSize.y - 1.0)));
            wsum += w;
        }
    }
    return sum / wsum;
}

// Phosphor mask: vertical R, G, B stripes, one device pixel each, scaled
// so its average is 1.
vec3 mask(vec2 frag) {
    float sub = mod(floor(frag.x), 3.0);
    vec3 m = vec3(MASK_DARK);
    if (sub < 0.5) {
        m.r = 1.0;
    } else if (sub < 1.5) {
        m.g = 1.0;
    } else {
        m.b = 1.0;
    }
    return m * 3.0 / (1.0 + 2.0 * MASK_DARK);
}

// Barrel distortion of the picture coordinates (crt-lottes' Warp).
vec2 warp(vec2 p) {
    p = p * 2.0 - 1.0;
    p *= vec2(1.0 + p.y * p.y * WARP.x, 1.0 + p.x * p.x * WARP.y);
    return p * 0.5 + 0.5;
}

// 1 on the lit tube face, 0 beyond its rounded edge, antialiased over a
// device pixel.
float face(vec2 p) {
    vec2 q = abs(p - 0.5) * vec2(4.0 / 3.0, 1.0);
    vec2 inner = vec2(2.0 / 3.0, 0.5) - CORNER;
    float d = length(max(q - inner, 0.0)) - CORNER;
    float aa = 1.0 / OutputSize.y;
    return 1.0 - smoothstep(-aa, aa, d);
}

void main() {
    vec2 p = warp(uv);
    // How much of the line and mask structure fits: none below 1.5 device
    // pixels per line, all from 3.
    float structure = clamp((OutputSize.y / SourceSize.y - 1.5) / 1.5, 0.0, 1.0);

    vec2 s = p * SourceSize;
    float row = floor(s.y - 0.5);
    float d = s.y - 0.5 - row;
    vec3 above = scan_line(row, s.x);
    vec3 below = scan_line(row + 1.0, s.x);
    vec3 lines = beam(above, d) + beam(below, 1.0 - d);
    vec3 flat_line = d < 0.5 ? above : below;
    vec3 c = mix(flat_line, lines, structure);

    c += GLOW * glow(s);
    c *= mix(vec3(1.0), mask(gl_FragCoord.xy), structure);
    float v = 16.0 * p.x * p.y * (1.0 - p.x) * (1.0 - p.y);
    c *= pow(clamp(v, 0.0, 1.0), VIGNETTE);

    c = pow(clamp(c, 0.0, 1.0), vec3(1.0 / GAMMA));
    gl_FragColor = vec4(c * face(p), 1.0);
}

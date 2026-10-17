#version 100
// Classic video (SPEC §13.1): passes the picture quad through unchanged.
// macroquad's default vertex shader, minus the vertex colour.

attribute vec3 position;
attribute vec2 texcoord;

varying vec2 uv;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1.0);
    uv = texcoord;
}

const vertexSource = `
attribute vec2 position;
varying vec2 uv;
void main() {
  uv = vec2((position.x + 1.0) * 0.5, (1.0 - position.y) * 0.5);
  gl_Position = vec4(position, 0.0, 1.0);
}`;

const pixelSource = `
precision mediump float;
varying vec2 uv;
uniform sampler2D Texture;
void main() { gl_FragColor = texture2D(Texture, uv); }
`;

function resize(canvas) {
  const ratio = Math.min(window.devicePixelRatio || 1, 2);
  const width = Math.max(1, Math.round(canvas.clientWidth * ratio));
  const height = Math.max(1, Math.round(canvas.clientHeight * ratio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
}

function createPixelRenderer(canvas) {
  const context = canvas.getContext("2d", { alpha: false });
  if (!context)
    throw new Error(
      "This browser cannot display the game. Try a browser with Canvas support.",
    );
  const source = document.createElement("canvas");
  const sourceContext = source.getContext("2d");
  let frame;
  return {
    draw(rgba, width, height) {
      resize(canvas);
      if (!frame || source.width !== width || source.height !== height) {
        source.width = width;
        source.height = height;
        frame = sourceContext.createImageData(width, height);
      }
      frame.data.set(rgba);
      sourceContext.putImageData(frame, 0, 0);
      context.imageSmoothingEnabled = false;
      context.drawImage(source, 0, 0, canvas.width, canvas.height);
    },
    dispose() {
      source.width = source.height = 0;
    },
  };
}

export async function createRenderer(canvas) {
  const gl = canvas.getContext("webgl", {
    alpha: false,
    antialias: false,
    depth: false,
    stencil: false,
  });
  if (!gl) return createPixelRenderer(canvas);

  function compile(type, text) {
    const shader = gl.createShader(type);
    gl.shaderSource(shader, text);
    gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
      const message = gl.getShaderInfoLog(shader);
      gl.deleteShader(shader);
      throw new Error(message || "Unable to compile the game display shader.");
    }
    return shader;
  }
  function program(fragmentSource) {
    const shaders = [];
    const result = gl.createProgram();
    try {
      shaders.push(compile(gl.VERTEX_SHADER, vertexSource));
      shaders.push(compile(gl.FRAGMENT_SHADER, fragmentSource));
      for (const shader of shaders) gl.attachShader(result, shader);
      gl.bindAttribLocation(result, 0, "position");
      gl.linkProgram(result);
      if (!gl.getProgramParameter(result, gl.LINK_STATUS)) {
        throw new Error(
          gl.getProgramInfoLog(result) ||
            "Unable to link the game display shader.",
        );
      }
      return {
        handle: result,
        texture: gl.getUniformLocation(result, "Texture"),
        source: gl.getUniformLocation(result, "SourceSize"),
        output: gl.getUniformLocation(result, "OutputSize"),
      };
    } catch (error) {
      gl.deleteProgram(result);
      throw error;
    } finally {
      for (const shader of shaders) gl.deleteShader(shader);
    }
  }

  const pixel = program(pixelSource);
  let classic = pixel;
  try {
    const response = await fetch("./crt.frag");
    if (!response.ok)
      throw new Error(`CRT shader request failed (${response.status})`);
    classic = program(await response.text());
  } catch (error) {
    console.warn("CRT display unavailable; using pixel display.", error);
  }

  const vertices = gl.createBuffer();
  gl.bindBuffer(gl.ARRAY_BUFFER, vertices);
  gl.bufferData(
    gl.ARRAY_BUFFER,
    new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]),
    gl.STATIC_DRAW,
  );
  gl.enableVertexAttribArray(0);
  gl.vertexAttribPointer(0, 2, gl.FLOAT, false, 0, 0);
  const texture = gl.createTexture();
  gl.activeTexture(gl.TEXTURE0);
  gl.bindTexture(gl.TEXTURE_2D, texture);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.NEAREST);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE);
  gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
  let sourceWidth = 0;
  let sourceHeight = 0;
  return {
    draw(rgba, width, height, crt = true) {
      if (gl.isContextLost())
        throw new Error(
          "The graphics context was lost. Reload to reconnect the game.",
        );
      resize(canvas);
      gl.viewport(0, 0, canvas.width, canvas.height);
      const selected = crt ? classic : pixel;
      gl.useProgram(selected.handle);
      gl.uniform1i(selected.texture, 0);
      if (selected.source !== null)
        gl.uniform2f(selected.source, width, height);
      if (selected.output !== null)
        gl.uniform2f(selected.output, canvas.width, canvas.height);
      if (width !== sourceWidth || height !== sourceHeight) {
        gl.texImage2D(
          gl.TEXTURE_2D,
          0,
          gl.RGBA,
          width,
          height,
          0,
          gl.RGBA,
          gl.UNSIGNED_BYTE,
          rgba,
        );
        sourceWidth = width;
        sourceHeight = height;
      } else {
        gl.texSubImage2D(
          gl.TEXTURE_2D,
          0,
          0,
          0,
          width,
          height,
          gl.RGBA,
          gl.UNSIGNED_BYTE,
          rgba,
        );
      }
      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);
    },
    dispose() {
      gl.deleteBuffer(vertices);
      gl.deleteTexture(texture);
      gl.deleteProgram(pixel.handle);
      if (classic !== pixel) gl.deleteProgram(classic.handle);
    },
  };
}

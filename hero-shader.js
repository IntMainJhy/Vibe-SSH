const vertexSource = `
  attribute vec2 a_position;
  varying mediump vec2 v_uv;

  void main() {
    v_uv = a_position * vec2(0.5, -0.5) + 0.5;
    gl_Position = vec4(a_position, 0.0, 1.0);
  }
`;

const fragmentSource = `
  precision mediump float;
  varying vec2 v_uv;
  uniform float u_phase;
  uniform float u_dark;

  float band(float distance, float width) {
    float offset = distance / width;
    return exp2(-offset * offset);
  }

  void main() {
    float x = v_uv.x;
    float y = v_uv.y;
    float sweep = 0.68 - 0.51 * x + 0.13 * sin(4.8 * x + u_phase);
    float echo = 0.43 + 0.30 * x + 0.10 * sin(5.7 * x - u_phase + 0.9);
    float fold = y - sweep;
    float backFold = y - echo;

    // The reflected edge is narrower than the shadow underneath it.
    float light = 0.74 * band(fold - 0.023, 0.027) + 0.33 * band(backFold + 0.034, 0.035);
    float shadow = 0.56 * band(fold + 0.039, 0.105) + 0.38 * band(backFold - 0.04, 0.13);
    float shade = mix(0.64, 1.0, clamp(light, 0.0, 1.0));
    float glow = 0.10 + 0.46 * light + 0.10 * shadow;
    float edges = smoothstep(0.0, 0.16, y) * (1.0 - smoothstep(0.67, 1.0, y));
    float sides = mix(0.35, 1.0, smoothstep(0.08, 0.48, abs(x - 0.5)));
    float opacity = clamp(0.42 * shadow + 0.51 * light, 0.0, 0.52) * edges * sides;

    gl_FragColor = vec4(vec3(mix(shade, glow, u_dark)), opacity);
  }
`;

function compileShader(gl, type, source) {
  const shader = gl.createShader(type);
  if (!shader) return null;
  gl.shaderSource(shader, source);
  gl.compileShader(shader);
  if (gl.getShaderParameter(shader, gl.COMPILE_STATUS)) return shader;
  gl.deleteShader(shader);
  return null;
}

function createRenderer(gl, canvas) {
  const vertex = compileShader(gl, gl.VERTEX_SHADER, vertexSource);
  const fragment = compileShader(gl, gl.FRAGMENT_SHADER, fragmentSource);
  const program = vertex && fragment ? gl.createProgram() : null;
  if (program) {
    gl.attachShader(program, vertex);
    gl.attachShader(program, fragment);
    gl.linkProgram(program);
  }
  if (vertex) gl.deleteShader(vertex);
  if (fragment) gl.deleteShader(fragment);
  if (!program) return null;

  const triangle = gl.getProgramParameter(program, gl.LINK_STATUS) && gl.createBuffer();
  if (!triangle) {
    gl.deleteProgram(program);
    return null;
  }

  gl.useProgram(program);
  gl.bindBuffer(gl.ARRAY_BUFFER, triangle);
  // One triangle covers the entire canvas.
  gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
  const position = gl.getAttribLocation(program, "a_position");
  const phaseUniform = gl.getUniformLocation(program, "u_phase");
  const darkUniform = gl.getUniformLocation(program, "u_dark");
  gl.enableVertexAttribArray(position);
  gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);

  return (width, height, phase, dark) => {
    if (canvas.width !== width || canvas.height !== height) {
      canvas.width = width;
      canvas.height = height;
    }
    gl.viewport(0, 0, gl.drawingBufferWidth, gl.drawingBufferHeight);
    gl.uniform1f(phaseUniform, phase);
    gl.uniform1f(darkUniform, Number(dark));
    gl.drawArrays(gl.TRIANGLES, 0, 3);
  };
}

const canvas = document.querySelector(".hero-shader");
const toggle = document.querySelector(".hero-motion");
const motion = matchMedia("(prefers-reduced-motion: reduce)");
const theme = matchMedia("(prefers-color-scheme: dark)");
const period = 24_000;
const pixelBudget = 550_000;

let gl;
let render = null;
let visible = false;
let paused = false;
let needsPaint = true;
let frame = 0;
let previousPaint = null;
let elapsed = 0;
let width = 0;
let height = 0;

function paint() {
  render(width, height, (elapsed / period) * 2 * Math.PI, theme.matches);
  needsPaint = false;
  if (toggle.hidden) {
    canvas.dataset.rendered = "";
    toggle.hidden = false;
  }
}

function animate(now) {
  const delta = previousPaint === null ? 0 : now - previousPaint;
  // Cap drawing at about 30 fps and avoid a jump after a stalled frame.
  if (previousPaint === null || delta >= 32) {
    elapsed = (elapsed + Math.min(delta, 64)) % period;
    paint();
    previousPaint = now;
  }
  frame = requestAnimationFrame(animate);
}

function sync() {
  const canDraw = visible && !document.hidden && !motion.matches && width && height;
  if (canDraw && gl === undefined) {
    gl = canvas.getContext("webgl", {
      antialias: false,
      depth: false,
      failIfMajorPerformanceCaveat: true,
      powerPreference: "low-power",
      premultipliedAlpha: false,
    });
    if (gl) render = createRenderer(gl, canvas);
  }

  if (canDraw && render && !paused) {
    if (!frame) frame = requestAnimationFrame(animate);
  } else {
    cancelAnimationFrame(frame);
    frame = 0;
    previousPaint = null;
    if (canDraw && render && needsPaint) paint();
  }
}

new IntersectionObserver(([entry]) => {
  visible = entry.isIntersecting;
  sync();
}).observe(canvas.parentElement);

new ResizeObserver(([{ contentRect }]) => {
  const { width: cssWidth, height: cssHeight } = contentRect;
  const scale = Math.min(0.8, 1000 / cssWidth, Math.sqrt(pixelBudget / (cssWidth * cssHeight)));
  width = Math.floor(cssWidth * scale);
  height = Math.floor(cssHeight * scale);
  needsPaint = true;
  sync();
}).observe(canvas);

toggle.addEventListener("click", () => {
  paused = !paused;
  toggle.textContent = paused ? "播放背景动画" : "暂停背景动画";
  sync();
});
document.addEventListener("visibilitychange", sync);
motion.addEventListener("change", sync);
theme.addEventListener("change", () => {
  needsPaint = true;
  sync();
});

canvas.addEventListener("webglcontextlost", (event) => {
  event.preventDefault();
  render = null;
  delete canvas.dataset.rendered;
  toggle.hidden = true;
  sync();
});
canvas.addEventListener("webglcontextrestored", () => {
  render = createRenderer(gl, canvas);
  needsPaint = true;
  sync();
});

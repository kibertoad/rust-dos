// Run with: node web/tests/screen.mjs /path/to/chrome-headless-shell
// Uses the page's actual upload and draw functions in a real browser.
import { readFileSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';
import { spawnSync } from 'node:child_process';

const browser = process.argv[2];
if (!browser) {
  throw new Error('Pass the headless Chromium executable as the argument.');
}
const source = readFileSync(new URL('../www/main.js', import.meta.url), 'utf8');
const draw = source.slice(source.indexOf('function draw() {'), source.indexOf('/// The screen drawn with WebGL 2:'));
const start = source.indexOf('function makeGlScreen() {');
const make = source.slice(start, source.indexOf('// The machine\'s frames', start));
const probe = `
let canvas = document.createElement('canvas');
document.body.append(canvas);
let machine = null;
let screenGl = null;
let screen2d = null;
let shownLook;
const wasm = { memory: { buffer: new ArrayBuffer(4096) } };
function layout() {}
function toast(message) { throw new Error(message); }
function showPicture() { screenGl.select('none', false, false, [0, 0], 0); }
function shader_program() {
  return [
    '#version 300 es\\nvoid main(){vec2 p=vec2((gl_VertexID<<1)&2,gl_VertexID&2);gl_Position=vec4(p*2.0-1.0,0,1);}',
    '#version 300 es\\nprecision highp float;out vec4 c;void main(){c=vec4(1);}'
  ];
}
function check(ok, message) { if (!ok) throw new Error(message); }
function picture(width, height, value) {
  return Uint8Array.from({length: width * height * 4}, (_, i) => i % 4 === 3 ? 255 : value + i % 3);
}
function equal(a, b, message) { check(a.length === b.length && a.every((v, i) => v === b[i]), message); }
function setMachine(width, height, pixels, first, end) {
  new Uint8Array(wasm.memory.buffer).set(pixels, 16);
  machine = {
    screen_width: () => width, screen_height: () => height,
    screen_pixels: () => 16, screen_row_start: () => first, screen_row_end: () => end,
  };
}
${draw}
${make}
async function run() {
  canvas.width = 8;
  canvas.height = 6;
  const gl = canvas.getContext('webgl2', { alpha: false, antialias: false, depth: false, stencil: false });
  check(gl, 'WebGL 2 unavailable');
  const uploads = [];
  const images = [];
  let mipmaps = 0;
  const sub = gl.texSubImage2D.bind(gl);
  gl.texSubImage2D = (...args) => { uploads.push(args); return sub(...args); };
  const image = gl.texImage2D.bind(gl);
  gl.texImage2D = (...args) => { images.push(args); return image(...args); };
  const mipmap = gl.generateMipmap.bind(gl);
  gl.generateMipmap = (...args) => { mipmaps++; return mipmap(...args); };
  screenGl = makeGlScreen();
  showPicture();
  function texture(width, height) {
    const framebuffer = gl.createFramebuffer();
    gl.bindFramebuffer(gl.FRAMEBUFFER, framebuffer);
    gl.framebufferTexture2D(gl.FRAMEBUFFER, gl.COLOR_ATTACHMENT0, gl.TEXTURE_2D, gl.getParameter(gl.TEXTURE_BINDING_2D), 0);
    check(gl.checkFramebufferStatus(gl.FRAMEBUFFER) === gl.FRAMEBUFFER_COMPLETE, 'incomplete texture');
    const result = new Uint8Array(width * height * 4);
    gl.readPixels(0, 0, width, height, gl.RGBA, gl.UNSIGNED_BYTE, result);
    gl.bindFramebuffer(gl.FRAMEBUFFER, null);
    gl.deleteFramebuffer(framebuffer);
    check(gl.getError() === gl.NO_ERROR, 'WebGL error');
    return result;
  }
  let pixels = picture(8, 6, 23);
  check(screenGl.upload(pixels, 8, 6, 2, 3), 'first upload must allocate');
  equal(texture(8, 6), pixels, 'first upload must fill all rows');
  check(uploads.at(-1)[3] === 0 && uploads.at(-1)[5] === 6, 'first upload extent');
  pixels.set(picture(8, 2, 60), 8 * 2 * 4);
  check(!screenGl.upload(pixels, 8, 6, 2, 4), 'same-size upload must reuse storage');
  check(images.length === 1, 'same-size texture reallocated');
  check(uploads.at(-1)[3] === 2 && uploads.at(-1)[5] === 2 && uploads.at(-1)[8].length === 64, 'partial upload extent');
  equal(texture(8, 6), pixels, 'partial upload corrupted other rows');
  const count = uploads.length;
  screenGl.upload(pixels, 8, 6, 0, 0);
  check(uploads.length === count, 'unchanged frame uploaded');
  screenGl.select('crt', false, false, [0, 0], 0);
  const priorMipmaps = mipmaps;
  pixels.set(picture(8, 1, 90), 8 * 5 * 4);
  screenGl.upload(pixels, 8, 6, 5, 6);
  check(mipmaps === priorMipmaps + 1, 'changed shader texture needs mipmaps');
  equal(texture(8, 6), pixels, 'bottom-row upload');
  pixels = picture(9, 4, 130);
  check(screenGl.upload(pixels, 9, 4, 1, 2), 'resize must allocate');
  equal(texture(9, 4), pixels, 'resize must fill all rows');
  setMachine(9, 4, pixels, 1, 2);
  const extension = gl.getExtension('WEBGL_lose_context');
  check(extension, 'context-loss extension unavailable');
  await new Promise(resolve => {
    canvas.addEventListener('webglcontextlost', () => setTimeout(() => { extension.restoreContext(); }, 10), {once: true});
    canvas.addEventListener('webglcontextrestored', resolve, {once: true});
    extension.loseContext();
  });
  equal(texture(9, 4), pixels, 'restored context must upload the complete cache');
  check(uploads.at(-1)[3] === 0 && uploads.at(-1)[5] === 4, 'restoration extent');

  canvas = document.createElement('canvas');
  document.body.append(canvas);
  screenGl = null;
  screen2d = canvas.getContext('2d');
  pixels = picture(7, 5, 31);
  setMachine(7, 5, pixels, 2, 3);
  draw();
  equal(screen2d.getImageData(0, 0, 7, 5).data, pixels, 'Canvas resize needs a full repaint');
  pixels.set(picture(7, 1, 91), 7 * 4 * 4);
  // A new memory buffer simulates WebAssembly memory growth.
  wasm.memory.buffer = new ArrayBuffer(8192);
  setMachine(7, 5, pixels, 4, 5);
  draw();
  equal(screen2d.getImageData(0, 0, 7, 5).data, pixels, 'Canvas partial upload and memory growth');
  setMachine(7, 5, pixels, 0, 0);
  draw();
  equal(screen2d.getImageData(0, 0, 7, 5).data, pixels, 'Canvas unchanged frame');
  return 'PASS: WebGL partial/unchanged/resize/restore/mipmap and Canvas partial/resize/memory growth';
}
run().then(result => { document.querySelector('#result').textContent = result; })
  .catch(error => { document.querySelector('#result').textContent = 'FAIL: ' + error.stack; });
`;
const directory = mkdtempSync(join(tmpdir(), 'rust-dos-web-screen-'));
const html = join(directory, 'screen.html');
writeFileSync(html, '<pre id="result">PENDING</pre><script>' + probe + '</script>');
const result = spawnSync(browser, [
  '--headless', '--no-first-run',
  '--use-gl=angle', '--use-angle=swiftshader', '--enable-unsafe-swiftshader',
  '--dump-dom', '--virtual-time-budget=5000',
  '--user-data-dir=' + join(directory, 'profile'), pathToFileURL(html).href,
], { encoding: 'utf8', windowsHide: true, timeout: 30000, maxBuffer: 2 << 20 });
const status = result.stdout?.match(/<pre id="result">([\s\S]*?)<\/pre>/)?.[1];
if (result.error || result.status !== 0 || !status?.startsWith('PASS:')) {
  throw new Error((result.error?.message ?? status ?? result.stderr) + '\nProbe: ' + html);
}
console.log(status);

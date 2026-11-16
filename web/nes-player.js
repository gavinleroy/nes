const KEYS = { KeyZ: 0x01, KeyX: 0x02, ShiftLeft: 0x04, ShiftRight: 0x04, Enter: 0x08, ArrowUp: 0x10, ArrowDown: 0x20, ArrowLeft: 0x40, ArrowRight: 0x80 };
const LEAD = 0.08;
const MIN_LATENCY = 0.03;

async function instantiate() {
  const url = new URL("nes.wasm", import.meta.url);
  try {
    return (await WebAssembly.instantiateStreaming(fetch(url))).instance;
  } catch {
    const bytes = await (await fetch(url)).arrayBuffer();
    return (await WebAssembly.instantiate(bytes)).instance;
  }
}

class NesPlayer extends HTMLElement {
  #wasm = null;
  #ctx;
  #image;
  #audio = null;
  #nextAudioTime = 0;
  #running = false;
  #buttons = 0;
  #overlay;

  constructor() {
    super();
    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `
      <style>
        :host { display: inline-block; position: relative; }
        canvas { display: block; image-rendering: pixelated; width: 100%; height: 100%; }
        div { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; cursor: pointer; }
        div[hidden] { display: none; }
      </style>
      <canvas part="canvas" width="256" height="240"></canvas>
      <div part="overlay"><slot>play</slot></div>`;
    this.#ctx = root.querySelector("canvas").getContext("2d");
    this.#image = this.#ctx.createImageData(256, 240);
    this.#overlay = root.querySelector("div");
    this.#overlay.addEventListener("click", () => this.start());
    this.addEventListener("keydown", e => this.#key(e, true));
    this.addEventListener("keyup", e => this.#key(e, false));
  }

  async connectedCallback() {
    if (!this.hasAttribute("tabindex")) {
      this.tabIndex = 0;
    }
    try {
      this.#wasm = (await instantiate()).exports;
      const src = this.getAttribute("src") ?? "rom.nes";
      const rom = new Uint8Array(await (await fetch(src)).arrayBuffer());
      const ptr = this.#wasm.alloc(rom.length);
      new Uint8Array(this.#wasm.memory.buffer, ptr, rom.length).set(rom);
      if (this.#wasm.load_rom(ptr, rom.length) !== 0) {
        throw new Error("unsupported rom");
      }
      this.dispatchEvent(new CustomEvent("nes-ready"));
    } catch (error) {
      this.#wasm = null;
      this.dispatchEvent(new CustomEvent("nes-error", { detail: error }));
    }
  }

  disconnectedCallback() {
    this.stop();
  }

  #key(event, down) {
    const bit = KEYS[event.code];
    if (!bit || !this.#running) {
      return;
    }
    this.#buttons = down ? this.#buttons | bit : this.#buttons & ~bit;
    event.preventDefault();
  }

  #emulateFrame() {
    this.#wasm.set_buttons(0, this.#buttons);
    this.#wasm.run_frame();
    const samples = new Float32Array(this.#wasm.memory.buffer, this.#wasm.samples_ptr(), this.#wasm.samples_len());
    if (samples.length === 0) {
      return;
    }
    const buffer = this.#audio.createBuffer(1, samples.length, 44100);
    buffer.copyToChannel(samples, 0);
    const source = this.#audio.createBufferSource();
    source.buffer = buffer;
    source.connect(this.#audio.destination);
    if (this.#nextAudioTime < this.#audio.currentTime + MIN_LATENCY) {
      this.#nextAudioTime = this.#audio.currentTime + MIN_LATENCY;
    }
    source.start(this.#nextAudioTime);
    this.#nextAudioTime += samples.length / 44100;
  }

  #tick = () => {
    if (!this.#running) {
      return;
    }
    let budget = 4;
    while (budget-- > 0 && this.#nextAudioTime - this.#audio.currentTime < LEAD) {
      this.#emulateFrame();
    }
    const pixels = new Uint8Array(this.#wasm.memory.buffer, this.#wasm.frame_ptr(), this.#wasm.frame_len() * 4);
    this.#image.data.set(pixels);
    this.#ctx.putImageData(this.#image, 0, 0);
    requestAnimationFrame(this.#tick);
  };

  start() {
    if (!this.#wasm || this.#running) {
      return;
    }
    if (!this.#audio) {
      this.#audio = new AudioContext({ sampleRate: 44100 });
    }
    this.#audio.resume();
    this.#nextAudioTime = 0;
    this.#running = true;
    this.#overlay.hidden = true;
    this.focus();
    requestAnimationFrame(this.#tick);
  }

  stop() {
    this.#running = false;
    this.#buttons = 0;
    this.#overlay.hidden = false;
  }

  reset() {
    this.#wasm?.reset();
  }

  setButtons(pad, bits) {
    this.#wasm?.set_buttons(pad, bits);
  }

  saveState() {
    if (!this.#wasm) {
      return null;
    }
    const len = this.#wasm.save_state();
    return new Uint8Array(this.#wasm.memory.buffer, this.#wasm.state_ptr(), len).slice();
  }

  loadState(bytes) {
    if (!this.#wasm) {
      return false;
    }
    const ptr = this.#wasm.alloc(bytes.length);
    new Uint8Array(this.#wasm.memory.buffer, ptr, bytes.length).set(bytes);
    return this.#wasm.load_state(ptr, bytes.length) === 0;
  }

  get running() {
    return this.#running;
  }
}

customElements.define("nes-player", NesPlayer);
export default NesPlayer;

const KEYS = { KeyX: 0x01, KeyZ: 0x02, ShiftLeft: 0x04, ShiftRight: 0x04, Enter: 0x08, ArrowUp: 0x10, ArrowDown: 0x20, ArrowLeft: 0x40, ArrowRight: 0x80 };
const LEAD = 0.08;
const PUMP_MS = 8;
const PUMP_BUDGET = 6;

const WORKLET_URL = URL.createObjectURL(
  new Blob(
    [
      `registerProcessor("nes-audio", class extends AudioWorkletProcessor {
        constructor() {
          super();
          this.queue = [];
          this.offset = 0;
          this.port.onmessage = e => this.queue.push(e.data);
        }
        process(inputs, outputs) {
          const out = outputs[0][0];
          let n = 0;
          while (n < out.length && this.queue.length) {
            const chunk = this.queue[0];
            const take = Math.min(out.length - n, chunk.length - this.offset);
            out.set(chunk.subarray(this.offset, this.offset + take), n);
            n += take;
            this.offset += take;
            if (this.offset === chunk.length) {
              this.queue.shift();
              this.offset = 0;
            }
          }
          return true;
        }
      });`,
    ],
    { type: "application/javascript" }
  )
);

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
  #node = null;
  #audioStart = 0;
  #sentSamples = 0;
  #running = false;
  #buttons = 0;
  #overlay;
  #pump = null;

  constructor() {
    super();
    const root = this.attachShadow({ mode: "open" });
    root.innerHTML = `
      <style>
        :host { display: inline-block; position: relative; }
        canvas { display: block; image-rendering: pixelated; width: 100%; height: 100%; object-fit: contain; }
        div { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; cursor: pointer; }
        [hidden] { display: none; }
        button { position: absolute; top: 4px; right: 4px; cursor: pointer; }
        aside { position: absolute; inset: 0; display: flex; align-items: center; justify-content: center; white-space: pre-line; }
      </style>
      <canvas part="canvas" width="256" height="240"></canvas>
      <div part="overlay"><slot>play</slot></div>
      <aside part="controls" hidden>x — A\nz — B\narrows — d-pad\nenter — start\nshift — select\ndouble-click — fullscreen</aside>
      <button part="help" type="button">?</button>`;
    this.#ctx = root.querySelector("canvas").getContext("2d");
    this.#image = this.#ctx.createImageData(256, 240);
    this.#overlay = root.querySelector("div");
    this.#overlay.addEventListener("click", () => this.start());
    const controls = root.querySelector("aside");
    root.querySelector("button").addEventListener("click", () => {
      controls.hidden = !controls.hidden;
    });
    this.addEventListener("dblclick", () => {
      if (document.fullscreenElement === this) {
        document.exitFullscreen();
      } else {
        this.requestFullscreen?.();
      }
    });
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

  #queuedSeconds() {
    const queued = this.#sentSamples / 44100 - (this.#audio.currentTime - this.#audioStart);
    if (queued < 0) {
      this.#audioStart += -queued;
      return 0;
    }
    return queued;
  }

  #emulate = () => {
    if (!this.#running) {
      return;
    }
    let budget = PUMP_BUDGET;
    while (budget-- > 0 && this.#queuedSeconds() < LEAD) {
      this.#wasm.set_buttons(0, this.#buttons);
      this.#wasm.run_frame();
      const samples = new Float32Array(this.#wasm.memory.buffer, this.#wasm.samples_ptr(), this.#wasm.samples_len()).slice();
      if (samples.length > 0) {
        this.#sentSamples += samples.length;
        this.#node.port.postMessage(samples, [samples.buffer]);
      }
    }
  };

  #draw = () => {
    if (!this.#running) {
      return;
    }
    const pixels = new Uint8Array(this.#wasm.memory.buffer, this.#wasm.frame_ptr(), this.#wasm.frame_len() * 4);
    this.#image.data.set(pixels);
    this.#ctx.putImageData(this.#image, 0, 0);
    requestAnimationFrame(this.#draw);
  };

  async start() {
    if (!this.#wasm || this.#running) {
      return;
    }
    if (!this.#audio) {
      this.#audio = new AudioContext({ sampleRate: 44100 });
      await this.#audio.audioWorklet.addModule(WORKLET_URL);
      this.#node = new AudioWorkletNode(this.#audio, "nes-audio", { outputChannelCount: [1] });
      this.#node.connect(this.#audio.destination);
    }
    await this.#audio.resume();
    this.#audioStart = this.#audio.currentTime;
    this.#sentSamples = 0;
    this.#running = true;
    this.#overlay.hidden = true;
    this.focus();
    this.#pump = setInterval(this.#emulate, PUMP_MS);
    requestAnimationFrame(this.#draw);
  }

  stop() {
    this.#running = false;
    this.#buttons = 0;
    this.#overlay.hidden = false;
    if (this.#pump !== null) {
      clearInterval(this.#pump);
      this.#pump = null;
    }
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

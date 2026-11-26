import { loadGtcrnModule } from "@sapphi-red/gtcrn-wasm";
import { createProcessor } from "gtcrn-frame-processor";

// Both imports are bundled from pinned source in pack-runtime.mjs.
class GtcrnWorklet extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.stopped = false;
    this.port.onmessage = ({ data }) => {
      if (data === "destroy") {
        this.stopped = true;
        this.processor?.destroy();
        this.processor = undefined;
      }
    };
    const { wasmBinary, maxChannels } = options.processorOptions;
    loadGtcrnModule({ wasmBinary, locateFile: file => file }).then(module => {
      if (!this.stopped) {
        this.processor = createProcessor(module, { bufferSize: 128, maxChannels, sampleRate });
      }
    });
  }

  process(inputs, outputs) {
    if (this.stopped) return false;
    if (inputs[0]?.length && outputs[0]?.length) this.processor?.process(inputs[0], outputs[0]);
    return true;
  }
}

registerProcessor("@sapphi-red/web-noise-suppressor/gtcrn", GtcrnWorklet);

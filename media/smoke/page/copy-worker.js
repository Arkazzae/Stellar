// The baselines that the cost of encrypted calls is measured against: the
// same encoded-transform hop through a worker, with no cryptography. Named
// "touch" it passes every frame on as it came; named "copy" it copies each
// frame's bytes once, as the encrypting worker must to assemble a frame.
// Only the smoke harness loads this file.

"use strict";

const copy = self.name === "copy";

function pipe(readable, writable) {
  readable
    .pipeThrough(
      new TransformStream({
        transform(frame, controller) {
          if (copy) frame.data = new Uint8Array(frame.data).slice().buffer;
          controller.enqueue(frame);
        },
      }),
    )
    .pipeTo(writable)
    .catch(() => {});
}

self.onrtctransform = (event) => pipe(event.transformer.readable, event.transformer.writable);

self.onmessage = ({ data }) => {
  if (data.type === "streams") pipe(data.readable, data.writable);
  else if (data.type === "stats") self.postMessage({ type: "stats", id: data.id, transforms: [] });
};

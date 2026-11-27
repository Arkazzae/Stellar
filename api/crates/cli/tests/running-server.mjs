// A hub connection that says when the server closes it.
//
//   node running-server.mjs --base http://127.0.0.1:5000 --cookie 'name=value'
//
// Prints `open` once the connection is made, then `closed <code> after <ms> ms`
// when the server ends it (or `still open` after --wait seconds). Needs Node 26
// (the global WebSocket takes request headers).

import { parseArgs } from "node:util";

const { values } = parseArgs({
  options: { base: { type: "string" }, cookie: { type: "string" }, wait: { type: "string", default: "20" } },
});
const socket = new WebSocket(`${values.base.replace("http", "ws")}/hubs/app`, {
  protocols: ["stellarr.v1"],
  headers: { Cookie: values.cookie, Origin: values.base },
});
let opened = 0;
socket.onmessage = ({ data }) => {
  if (JSON.parse(data).type === "ready") {
    opened = Date.now();
    console.log("open");
  }
};
socket.onerror = () => {
  console.log("could not connect");
  process.exit(2);
};
socket.onclose = (event) => {
  console.log(`closed ${event.code} after ${Date.now() - opened} ms`);
  process.exit(0);
};
setTimeout(() => {
  console.log("still open");
  process.exit(1);
}, Number(values.wait) * 1000);

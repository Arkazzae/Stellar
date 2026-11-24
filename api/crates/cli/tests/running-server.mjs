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
const RS = "\u001e";
const socket = new WebSocket(`${values.base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: values.cookie } });
let opened = 0;
socket.onopen = () => {
  socket.send(`{"protocol":"json","version":1}${RS}`);
  console.log("open");
  opened = Date.now();
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

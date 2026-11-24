// Listens on the hub for the people given and writes what each hears, one JSON
// line per event, until it is told to stop by closing its standard input.
//
//   node messages-listen.mjs <base> <out> name=cookie [name=cookie ...]
//
// Needs Node 26 (the global WebSocket takes request headers).
import { appendFileSync, writeFileSync } from "node:fs";

const [base, out, ...people] = process.argv.slice(2);
const RS = "\u001e";
writeFileSync(out, "");
const sockets = [];
for (const entry of people) {
  const at = entry.indexOf("=");
  const name = entry.slice(0, at);
  const cookie = entry.slice(at + 1);
  const socket = new WebSocket(`${base.replace("http", "ws")}/hubs/app`, { headers: { Cookie: cookie } });
  let buffer = "";
  socket.onmessage = (event) => {
    buffer += String(event.data);
    let end;
    while ((end = buffer.indexOf(RS)) >= 0) {
      const record = JSON.parse(buffer.slice(0, end));
      buffer = buffer.slice(end + 1);
      if (record.type === 1) appendFileSync(out, JSON.stringify({ who: name, target: record.target, arguments: record.arguments }) + "\n");
    }
  };
  await new Promise((resolve, reject) => {
    socket.onopen = resolve;
    socket.onerror = () => reject(new Error(`the upgrade for ${name} failed`));
  });
  socket.send(`{"protocol":"json","version":1}${RS}`);
  sockets.push(socket);
}
console.log("listening");
process.stdin.resume();
process.stdin.on("end", () => {
  for (const socket of sockets) socket.close();
  setTimeout(() => process.exit(0), 200);
});

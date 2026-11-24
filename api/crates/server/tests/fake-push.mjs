// A push service for comparing the two servers.
//
//   node fake-push.mjs <port> <cert.pem> <key.pem> <subscriptions.json> <vapid public key>
//
// It answers whatever the control path says, checks the signed header against
// the instance's public key, and opens the body with the keys of the
// subscription the path names, in either of the encodings the two servers
// use (`aesgcm`, the first draft, and `aes128gcm`, RFC 8291). Everything it
// saw is at /__log, and /__behave?... sets what it does next:
//
//   /__behave?mode=status&code=410[&retryAfter=30]   answer that status (default 201)
//   /__behave?mode=silent                            take the request, never answer
//   /__behave?mode=close-head                        start the answer and hang up
//   /__behave?mode=close-body                        send 201 and hang up in the body
//   /__behave?mode=big&bytes=5000000                 201 with a body of that size
//   /__behave?mode=redirect&to=https://...           307
//   &pass=<name> on /__behave                        stamps every request that arrives afterwards
//   /__reset                                         forget the log
//
// subscriptions.json: { "<path>": { "d": "<private key, base64url>", "x": ..., "y": ..., "auth": "<base64url>" } }
import https from 'node:https';
import fs from 'node:fs';
import crypto from 'node:crypto';

const [port, certFile, keyFile, subsFile, vapidPublic] = process.argv.slice(2);
const subs = JSON.parse(fs.readFileSync(subsFile, 'utf8'));
const log = [];
let behaviour = { mode: 'status', code: 201 };

const b64 = (text) => Buffer.from(text, 'base64url');
const hkdf = (salt, ikm, info, length) => Buffer.from(crypto.hkdfSync('sha256', ikm, salt, info, length));

function verifyVapid(headers) {
  let jwt, key;
  const authorization = headers['authorization'] ?? '';
  if (authorization.startsWith('WebPush ')) {
    jwt = authorization.slice(8).trim();
    key = /p256ecdsa=([^;,\s]+)/.exec(headers['crypto-key'] ?? '')?.[1];
  } else if (authorization.startsWith('vapid ')) {
    jwt = /t=([^,\s]+)/.exec(authorization)?.[1];
    key = /k=([^,\s]+)/.exec(authorization)?.[1];
  }
  if (!jwt || !key) return { ok: false, why: 'no token or key' };
  if (key !== vapidPublic) return { ok: false, why: 'the key is not the instance\'s' };
  const [head, claims, signature] = jwt.split('.');
  const point = b64(key);
  const publicKey = crypto.createPublicKey({ key: { kty: 'EC', crv: 'P-256', x: point.subarray(1, 33).toString('base64url'), y: point.subarray(33).toString('base64url') }, format: 'jwk' });
  const good = crypto.verify('sha256', Buffer.from(`${head}.${claims}`), { key: publicKey, dsaEncoding: 'ieee-p1363' }, b64(signature));
  const parsed = JSON.parse(b64(claims).toString());
  const now = Date.now() / 1000;
  return { ok: good && parsed.exp > now && parsed.exp <= now + 86400 + 60, header: JSON.parse(b64(head).toString()), claims: parsed };
}

function decrypt(path, headers, body) {
  const sub = subs[path];
  if (!sub) return { error: 'unknown subscription' };
  const receiver = crypto.createECDH('prime256v1');
  receiver.setPrivateKey(b64(sub.d));
  const uaPublic = receiver.getPublicKey();
  const auth = b64(sub.auth);
  const encoding = headers['content-encoding'];
  try {
    if (encoding === 'aes128gcm') {
      const salt = body.subarray(0, 16);
      const recordSize = body.readUInt32BE(16);
      const idLength = body[20];
      const sender = body.subarray(21, 21 + idLength);
      const record = body.subarray(21 + idLength);
      const secret = receiver.computeSecret(sender);
      const ikm = hkdf(auth, secret, Buffer.concat([Buffer.from('WebPush: info\0'), uaPublic, sender]), 32);
      const key = hkdf(salt, ikm, Buffer.from('Content-Encoding: aes128gcm\0'), 16);
      const nonce = hkdf(salt, ikm, Buffer.from('Content-Encoding: nonce\0'), 12);
      const decipher = crypto.createDecipheriv('aes-128-gcm', key, nonce);
      decipher.setAuthTag(record.subarray(record.length - 16));
      const plain = Buffer.concat([decipher.update(record.subarray(0, record.length - 16)), decipher.final()]);
      let end = plain.length - 1;
      while (end >= 0 && plain[end] === 0) end--;
      return { encoding, recordSize, text: plain.subarray(0, end).toString('utf8'), delimiter: plain[end] };
    }
    if (encoding === 'aesgcm') {
      const salt = b64(/salt=([^;,\s]+)/.exec(headers['encryption'] ?? '')[1]);
      const sender = b64(/dh=([^;,\s]+)/.exec(headers['crypto-key'] ?? '')[1]);
      const secret = receiver.computeSecret(sender);
      const prk = hkdf(auth, secret, Buffer.from('Content-Encoding: auth\0'), 32);
      const lengthOf = (buffer) => { const b = Buffer.alloc(2); b.writeUInt16BE(buffer.length); return b; };
      const context = Buffer.concat([Buffer.from('P-256\0'), lengthOf(uaPublic), uaPublic, lengthOf(sender), sender]);
      const key = hkdf(salt, prk, Buffer.concat([Buffer.from('Content-Encoding: aesgcm\0'), context]), 16);
      const nonce = hkdf(salt, prk, Buffer.concat([Buffer.from('Content-Encoding: nonce\0'), context]), 12);
      const decipher = crypto.createDecipheriv('aes-128-gcm', key, nonce);
      decipher.setAuthTag(body.subarray(body.length - 16));
      const plain = Buffer.concat([decipher.update(body.subarray(0, body.length - 16)), decipher.final()]);
      const padding = plain.readUInt16BE(0);
      return { encoding, recordSize: null, text: plain.subarray(2 + padding).toString('utf8') };
    }
    return { error: `unknown content encoding ${encoding}` };
  } catch (error) {
    return { error: String(error.message) };
  }
}

const server = https.createServer({ cert: fs.readFileSync(certFile), key: fs.readFileSync(keyFile) }, (request, response) => {
  const url = new URL(request.url, 'https://x');
  if (url.pathname === '/__log') { response.end(JSON.stringify(log)); return; }
  if (url.pathname === '/__reset') { log.length = 0; response.end('ok'); return; }
  if (url.pathname === '/__behave') { behaviour = Object.fromEntries(url.searchParams); response.end('ok'); return; }
  const chunks = [];
  request.on('data', (chunk) => chunks.push(chunk));
  request.on('end', () => {
    const body = Buffer.concat(chunks);
    const headers = request.headers;
    const entry = {
      method: request.method, path: url.pathname, httpVersion: request.httpVersion,
      headers: Object.fromEntries(Object.entries(headers).filter(([name]) => name !== 'authorization' && name !== 'crypto-key')),
      rawHeaderNames: request.rawHeaders.filter((_, index) => index % 2 === 0),
      bodyLength: body.length, vapid: verifyVapid(headers), message: decrypt(url.pathname, headers, body),
      behaviour: behaviour.mode, pass: behaviour.pass ?? null, at: Date.now(),
    };
    log.push(entry);
    switch (behaviour.mode) {
      case 'silent': return;
      case 'close-head': response.socket.write('HTTP/1.1 201 Cre'); response.socket.destroy(); return;
      case 'close-body': response.socket.write('HTTP/1.1 201 Created\r\nContent-Length: 1000\r\n\r\nabc'); response.socket.destroy(); return;
      case 'big': {
        const size = Number(behaviour.bytes ?? 5000000);
        response.writeHead(201, { 'Content-Length': size });
        response.end(Buffer.alloc(size, 0x78));
        return;
      }
      case 'redirect': response.writeHead(307, { Location: behaviour.to }); response.end(); return;
      default: {
        const extra = behaviour.retryAfter ? { 'Retry-After': behaviour.retryAfter } : {};
        response.writeHead(Number(behaviour.code ?? 201), extra);
        response.end();
      }
    }
  });
});
server.listen(Number(port), '127.0.0.1', () => console.log(`fake push service on ${port}`));

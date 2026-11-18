/**
 * The web client's one service worker. An origin's scope has room for one,
 * and a second script registered for it would replace the first, so a single
 * file, /stellarr-worker.js, serves Web Push and keeps the shell's local copy.
 * Everyone registers it at start; push enrolment registers it again, which
 * changes nothing.
 */
export const WORKER_PATH = "/stellarr-worker.js";
export const WORKER_SCOPE = "/";

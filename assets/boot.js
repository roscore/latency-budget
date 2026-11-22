// External bootstrap so the CSP needs no 'unsafe-inline' for scripts.
import init from './lb_web.js';

init({ module_or_path: new URL('./lb_web_bg.wasm', import.meta.url) });

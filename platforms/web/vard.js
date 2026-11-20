// Vard web host: runs a bundled Vard project in a <canvas> with the core and the GPU
// renderer compiled to WebAssembly (crates/vard-web). Real <input> elements float over the
// app's Inputs; pointer, wheel and keyboard events go to the core; the std platform is
// answered with browser APIs; requestAnimationFrame runs only while something animates.
//
//   import { start } from "./vard.js";
//   const vard = await start(document.getElementById("app"), { base: "./app/" });
//   vard.run('tap "Add"; tree')   // the `vard run --do` action language, from the console
import init, { VardApp, createCanvas } from "./pkg/vard_web.js";

/** Fetch the bundle written by build.sh: files.json lists sources, fonts and assets. */
async function loadBundle(base) {
  const list = await (await fetch(base + "files.json")).json();
  const files = await Promise.all(list.sources.map(async (p) => [p, await (await fetch(base + p)).text()]));
  const fonts = await Promise.all(list.fonts.map(async (p) => [p.split("/").pop(), new Uint8Array(await (await fetch(base + p)).arrayBuffer())]));
  return { files, fonts };
}

/** The std `extern fn`s (crates/vard-lang/src/std.vard) with browser APIs. */
function stdModules(engine) {
  const store = window.localStorage;
  const cb = (arg, value) => queueMicrotask(() => engine.answer(arg, value));
  const permissionNames = { camera: "camera", microphone: "microphone", location: "geolocation", notifications: "notifications" };
  const known = {};
  for (const [p, name] of Object.entries(permissionNames)) {
    navigator.permissions?.query({ name }).then((s) => { known[p] = s.state; s.onchange = () => (known[p] = s.state); }).catch(() => {});
  }
  const status = (p) => {
    if (p === "notifications" && "Notification" in window) {
      return { granted: "granted", denied: "blocked" }[Notification.permission] || "notDetermined";
    }
    return { granted: "granted", denied: "blocked", prompt: "notDetermined" }[known[p]] || (permissionNames[p] ? "notDetermined" : "denied");
  };
  return {
    prefGet: ([k]) => store.getItem("vard.pref:" + k),
    prefSet: ([k, v]) => store.setItem("vard.pref:" + k, v),
    prefRemove: ([k]) => store.removeItem("vard.pref:" + k),
    // the browser has no keychain: kept in localStorage, readable by this origin's scripts
    secureGet: ([k]) => store.getItem("vard.secure:" + k),
    secureSet: ([k, v]) => (store.setItem("vard.secure:" + k, v), true),
    secureRemove: ([k]) => store.removeItem("vard.secure:" + k),
    openUrl: ([url]) => window.open(url, "_blank", "noopener"),
    shareText: ([text]) => (navigator.share ? navigator.share({ text }).catch(() => {}) : navigator.clipboard?.writeText(text)),
    haptic: ([kind]) => navigator.vibrate?.(kind === "heavy" || kind === "error" ? 30 : 10),
    openAppSettings: () => null,
    permissionStatus: ([p]) => status(p),
    requestPermission: ([p, done]) => {
      const finish = () => cb(done, status(p));
      if (p === "notifications" && "Notification" in window) Notification.requestPermission().then(finish, finish);
      else if (p === "camera" || p === "microphone")
        navigator.mediaDevices?.getUserMedia(p === "camera" ? { video: true } : { audio: true })
          .then((s) => { s.getTracks().forEach((t) => t.stop()); known[p] = "granted"; finish(); }, () => { known[p] = "denied"; finish(); });
      else if (p === "location")
        navigator.geolocation?.getCurrentPosition(() => { known[p] = "granted"; finish(); }, () => { known[p] = "denied"; finish(); });
      else cb(done, "denied");
      return null;
    },
    showNotification: ([title, body, data]) => {
      if ("Notification" in window && Notification.permission === "granted") new Notification(title, { body, data });
    },
    getPushToken: ([done]) => (cb(done, null), null),
  };
}

export async function start(root, { base = "./app/", view = null, modules = {} } = {}) {
  await init();
  const { files, fonts } = await loadBundle(base);
  root.style.position = "relative";
  root.style.overflow = "hidden";
  const canvas = document.createElement("canvas");
  canvas.style.cssText = "position:absolute;inset:0;width:100%;height:100%;touch-action:none;outline:none";
  canvas.tabIndex = 0;
  root.appendChild(canvas);
  const css = () => [root.clientWidth || 390, root.clientHeight || 844];
  const [w, h] = css();
  const app = new VardApp(files, w, h);
  for (const [name, bytes] of fonts) app.addFont(name, bytes);

  const engine = {
    answer(arg, value) {
      const id = arg && arg.$callback;
      if (id != null && app.callback(id, JSON.stringify(value ?? null), false) === 1) frame();
    },
  };
  const table = { ...stdModules(engine), ...modules };
  app.setHost((name, argsJson) => {
    const f = table[name];
    if (!f) return undefined; // R0010 in the app
    const r = f(JSON.parse(argsJson), engine);
    return JSON.stringify(r === undefined || (r && typeof r.then === "function") ? null : r);
  });
  const dark = window.matchMedia("(prefers-color-scheme: dark)");
  app.setDark(dark.matches);
  app.setLocale(navigator.language || "en");
  try {
    app.mount(view);
  } catch (e) {
    root.innerHTML = `<pre style="color:#b00020;padding:12px;white-space:pre-wrap">${String(e.message || e).replace(/</g, "&lt;")}</pre>`;
    throw e;
  }

  const dpr = () => window.devicePixelRatio || 1;
  canvas.width = Math.round(w * dpr());
  canvas.height = Math.round(h * dpr());
  const gpu = await createCanvas(canvas, canvas.width, canvas.height, dpr());

  // ---- frames
  let scheduled = false, last = 0, lastBack = false;
  const inputs = new Map();
  const fetching = new Set();
  function frame() {
    if (scheduled) return;
    scheduled = true;
    requestAnimationFrame(tick);
  }
  function tick(t) {
    scheduled = false;
    if (last) app.advance(Math.min(100, t - last));
    const s = JSON.parse(gpu.render(app));
    if (!s) return;
    root.style.background = s.background.slice(0, 7);
    syncInputs(s.inputs);
    for (const src of s.images) loadImage(src);
    if (s.back && !lastBack) history.pushState({ vard: true }, "");
    lastBack = s.back;
    if (s.animating) { last = t; frame(); } else last = 0;
  }
  async function loadImage(src) {
    if (fetching.has(src)) return;
    fetching.add(src);
    try {
      const res = await fetch(/^[a-z]+:\/\//.test(src) ? src : base + src);
      gpu.addImage(src, new Uint8Array(await res.arrayBuffer()));
    } catch {
      gpu.addImage(src, new Uint8Array());
    }
    frame();
  }

  // ---- native <input> over each Input record
  function syncInputs(list) {
    const seen = new Set();
    for (const r of list) {
      seen.add(r.uid);
      let el = inputs.get(r.uid);
      if (!el) {
        el = document.createElement("input");
        el.style.cssText = "position:absolute;border:0;outline:0;background:transparent;padding:0;margin:0";
        el.addEventListener("input", () => { el.sent = el.value; app.inputText(r.uid, el.value); frame(); });
        el.addEventListener("focus", () => { app.focus(r.uid); frame(); });
        el.addEventListener("blur", () => { app.focus(0); frame(); });
        el.addEventListener("keydown", (e) => { if (e.key === "Enter") { app.submit(r.uid); frame(); } });
        root.appendChild(el);
        inputs.set(r.uid, el);
      }
      const type = r.secure ? "password" : r.keyboard;
      if (el.type !== type) el.type = type;
      // the app may rewrite the text itself (clear on submit, formatting): that wins
      if (el.value !== r.text && (document.activeElement !== el || r.text !== el.sent)) el.value = el.sent = r.text;
      el.placeholder = r.placeholder;
      el.disabled = r.disabled;
      Object.assign(el.style, {
        left: r.x + "px", top: r.y + "px", width: r.w + "px", height: r.h + "px",
        color: r.color, font: `${r.weight} ${r.fontPx}px ${r.font ? `"${r.font}", ` : ""}system-ui, sans-serif`,
        textAlign: r.align, opacity: r.opacity, letterSpacing: r.letterSpacing + "px",
        clipPath: `inset(${Math.max(0, r.clip[1] - r.y)}px ${Math.max(0, r.x + r.w - r.clip[0] - r.clip[2])}px ${Math.max(0, r.y + r.h - r.clip[1] - r.clip[3])}px ${Math.max(0, r.clip[0] - r.x)}px)`,
      });
      el.style.setProperty("--ph", r.placeholderColor);
    }
    for (const [uid, el] of inputs) if (!seen.has(uid)) { el.remove(); inputs.delete(uid); }
  }
  const style = document.createElement("style");
  style.textContent = "input::placeholder{color:var(--ph)}";
  document.head.appendChild(style);

  // ---- pointer: tap, or drag to scroll (with momentum)
  let down = null, dragging = false, lastY = 0, velocity = 0, lastMove = 0;
  canvas.addEventListener("pointerdown", (e) => {
    down = [e.offsetX, e.offsetY]; dragging = false; lastY = e.offsetY; velocity = 0; lastMove = e.timeStamp;
    canvas.setPointerCapture(e.pointerId);
  });
  canvas.addEventListener("pointermove", (e) => {
    if (!down) return;
    if (!dragging && Math.hypot(e.offsetX - down[0], e.offsetY - down[1]) > 6) dragging = true;
    if (dragging) {
      const dy = lastY - e.offsetY;
      velocity = dy / Math.max(1, e.timeStamp - lastMove);
      lastY = e.offsetY; lastMove = e.timeStamp;
      if (app.scroll(down[0], down[1], dy) === 1) frame();
    }
  });
  canvas.addEventListener("pointerup", (e) => {
    if (down && !dragging) {
      document.activeElement?.blur?.();
      app.tap(e.offsetX, e.offsetY);
      frame();
    } else if (down && Math.abs(velocity) > 0.2) {
      const at = down;
      let v = velocity * 16;
      const fling = () => {
        if (Math.abs(v) < 0.5 || app.scroll(at[0], at[1], v) !== 1) return;
        v *= 0.95; frame(); requestAnimationFrame(fling);
      };
      requestAnimationFrame(fling);
    }
    down = null;
  });
  canvas.addEventListener("wheel", (e) => {
    e.preventDefault();
    const dy = e.deltaMode === 1 ? e.deltaY * 40 : e.deltaY;
    if (app.scroll(e.offsetX, e.offsetY, dy) === 1) frame();
  }, { passive: false });

  // ---- back: Escape, the browser's back button while the app handles back
  window.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && app.backEnabled()) { app.back(); frame(); }
  });
  window.addEventListener("popstate", () => {
    if (app.backEnabled()) { app.back(); frame(); }
  });

  // ---- size, theme, language, lifecycle
  new ResizeObserver(() => {
    const [cw, ch] = css();
    canvas.width = Math.round(cw * dpr());
    canvas.height = Math.round(ch * dpr());
    gpu.resize(canvas.width, canvas.height, dpr());
    app.resize(cw, ch);
    frame();
  }).observe(root);
  dark.addEventListener("change", () => { app.setDark(dark.matches); frame(); });
  window.addEventListener("languagechange", () => { app.setLocale(navigator.language); frame(); });
  document.addEventListener("visibilitychange", () => {
    app.emit(document.hidden ? "appPause" : "appResume", "[]");
    frame();
  });
  const hash = () => location.hash.length > 1 && app.emit("deepLink", JSON.stringify([location.href]));
  window.addEventListener("hashchange", () => { hash(); frame(); });
  hash();

  frame();
  const diag = app.diagnostics();
  if (diag.trim()) console.warn(diag);
  return {
    app,
    backend: gpu.backend,
    /** Run actions (`tap "Add"; tree`) against the live app and redraw. */
    run(script) { const out = app.run(script); frame(); return out; },
    /** Deliver `extern event name(...)`. */
    emit(name, ...args) { app.emit(name, JSON.stringify(args)); frame(); },
    redraw: frame,
  };
}

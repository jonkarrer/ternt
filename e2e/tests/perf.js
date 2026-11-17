// Costs of a state update with a big session (e2e/tests/perf.session.json:
// 203 tabs in 3 spaces, 12 folders, 4 groups). Logs the numbers; the limits
// are loose, to catch regressions rather than measure.
const S = () => t.state();
await t.until(() => S().tabs.length === 203, "the big session is back");
await t.log(`sidebar page start to session shown: ${Math.round(performance.now())}ms`);
await t.sleep(1000);

// Rust: build the snapshot and serialize it
const [build, ser, bytes] = await t.invoke("debug_bench_snapshot", { n: 50 });
await t.log(`snapshot: build ${build.toFixed(3)}ms, serialize ${ser.toFixed(3)}ms, ${bytes} bytes`);

// Sidebar: apply and render an update (a page's title changed)
const base = JSON.parse(JSON.stringify(S()));
const times = [];
for (let i = 0; i < 40; i++) {
  const next = structuredClone(base);
  const tab = next.tabs.find((x) => x.id === next.active);
  tab.title = `Title ${i}`;
  const t0 = performance.now();
  t.apply(next);
  times.push(performance.now() - t0);
}
times.sort((a, b) => a - b);
const median = times[times.length >> 1];
await t.log(`sidebar update: median ${median.toFixed(2)}ms, max ${times.at(-1).toFixed(2)}ms`);

// Switching spaces re-renders the whole list
const switches = [];
for (let i = 0; i < 12; i++) {
  const next = structuredClone(base);
  next.activeSpace = [1, 2, 3][i % 3];
  const t0 = performance.now();
  t.apply(next);
  switches.push(performance.now() - t0);
}
switches.sort((a, b) => a - b);
await t.log(`space switch render: median ${switches[switches.length >> 1].toFixed(2)}ms`);
t.apply(base);

// End to end: Rust changes a title, the sidebar has it
const id = S().active;
let arrived = null;
const off = await t.listen("browser:state", (e) => arrived?.(e.payload));
const e2e = [];
for (let i = 0; i < 20; i++) {
  const got = new Promise((resolve) => (arrived = (p) => p.tabs.find((x) => x.id === id)?.name === `E2E ${i}` && resolve()));
  const t0 = performance.now();
  await t.invoke("rename_tab", { id, name: `E2E ${i}` });
  await got;
  e2e.push(performance.now() - t0);
}
off();
e2e.sort((a, b) => a - b);
await t.log(`rename, Rust to sidebar: median ${e2e[e2e.length >> 1].toFixed(1)}ms`);

await t.assert(median < 50, "a sidebar update renders in well under a frame budget's worth");
await t.quit();

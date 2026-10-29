const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const text = (id) => t.read("term", `window.__termText(${id})`);
const run = (id, cmd) => t.invoke("pty_write", { id, data: cmd + "\r" });
async function waitFor(id, needle, message, timeout = 10000) {
  const start = performance.now();
  for (;;) {
    const out = await text(id);
    if (out.includes(needle)) return t.log(`ok - ${message}`);
    if (performance.now() - start > timeout) throw new Error(`timed out: ${message}\n--- terminal ---\n${out}`);
    await t.sleep(150);
  }
}

// A web tab first, so we exercise switching between kinds.
await t.invoke("new_tab", { url: "http://localhost:8765/" });
await t.until(() => tab()?.title === "Fixture Home", "web tab loaded");
const webId = S().active;

// Open a terminal via its shortcut
await t.invoke("menu_action", { id: "new-terminal" });
await t.until(() => tab()?.kind === "terminal", "cmd+opt+t opens a terminal tab");
const term1 = S().active;
await t.sleep(1500); // login shell startup
await run(term1, "echo ternt-$((6*7))");
await waitFor(term1, "ternt-42", "shell runs commands and output streams back");

// Shell sets the tab title via OSC 2
await run(term1, "printf '\\033]2;build-logs\\007'");
await t.until(() => tab()?.title === "build-logs", "OSC title updates the tab");

// Resize reaches the PTY
await t.resize(1100, 720);
await t.sleep(400);
await run(term1, "echo size:$(stty size)");
// Wait for the output line, not the echoed command (which also says "size:").
const size = await t.until(async () => (await text(term1)).match(/size:(\d+) (\d+)/), "stty reports a size");
await t.assert(size && +size[1] > 10 && +size[2] > 40, `pty sized to the view (${size?.[1]} rows x ${size?.[2]} cols)`);

// Flood: 200k lines must not wedge anything (flow control)
const before = performance.now();
await run(term1, "seq 1 200000 && echo flood-done");
await waitFor(term1, "flood-done", "200k-line flood completes", 30000);
await t.log(`flood took ${Math.round(performance.now() - before)}ms`);

// Switch web <-> terminal
await t.invoke("activate_tab", { id: webId });
await t.until(() => S().active === webId, "switch back to web tab");
await t.shot("terminal-web-visible");
await t.invoke("activate_tab", { id: term1 });
await t.until(() => S().active === term1, "switch back to terminal");

// Second terminal is independent
await t.invoke("new_terminal");
await t.until(() => tab()?.kind === "terminal" && S().active !== term1, "second terminal");
const term2 = S().active;
await t.sleep(1500);
await run(term2, "echo second-$((2+2))");
await waitFor(term2, "second-4", "second terminal has its own shell");
await t.assert(!(await text(term2)).includes("flood-done"), "terminals don't share output");

// `exit` closes the tab
await run(term2, "exit");
await t.until(() => !S().tabs.some((x) => x.id === term2), "exiting the shell closes its tab");

// Closing a background terminal keeps the active one visible
await t.invoke("new_terminal");
await t.until(() => S().tabs.filter((x) => x.kind === "terminal").length === 2, "third terminal");
const term3 = S().active;
await t.invoke("activate_tab", { id: term1 });
await t.invoke("close_tab", { id: term3 });
await t.until(() => !S().tabs.some((x) => x.id === term3), "background terminal closed");
await t.sleep(300);
await t.shot("terminal-active");
await t.quit();

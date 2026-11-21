// The Tokidoki panel (⌘J) against the stub agent: per-space conversations,
// streamed replies, tool calls, failed turns, stopping, and how it shares
// the window with the command bar.
const S = () => t.state();
const A = t.agent;
const userRows = async () => (await A.rows()).filter(([k]) => k === "user");
const assistantRows = async () => (await A.rows()).filter(([k]) => k === "assistant");

const first = S().activeSpace;
await A.open();
const req = await A.request();
await t.assert(req.space.id === first && req.folder === `space-${first}`, "opens for the active space, folder by id");
await t.assert(["stub", "starting"].includes(req.backend), `the request names the backend (${req.backend})`);
await t.assert((await A.text("h1 .space")) === S().spaces.find((s) => s.id === first).name, "header names the space");
// The agent runs in a sidecar process, started with the app
const status = await t.until(async () => {
  const s = await t.invoke("agent_status");
  return s.state === "running" ? s : null;
}, "the agent process is running");
await t.assert(status.backend === "stub" && status.restarted === false, "it is the stub, on its first start");
await t.until(async () => (await A.text(".badge")) === "stub", "the panel shows the backend");
await t.shot("agent-empty");

// A question streams back an answer
await A.type("hello from the test");
await A.key("Enter");
await t.until(async () => (await userRows()).some(([, x]) => x === "hello from the test"), "the question shows as a user row");
await t.until(async () => (await A.text(".presence")).length > 0, "presence shows while it answers");
await t.until(async () => (await A.rows()).some(([k]) => k === "live"), "the reply streams in", 10000);
await t.shot("agent-streaming");
await t.until(async () => (await assistantRows()).some(([, x]) => x.includes("standing in until tokidoki")), "then lands as an assistant row", 15000);
await t.until(async () => !(await A.rows()).some(([k]) => k === "live") && !(await A.text(".presence")), "streaming and presence end with the turn");
await t.shot("agent-reply");

// A tool call shows with its result
await A.type("/tool what's open?");
await A.key("Enter");
await t.until(async () => (await A.rows()).some(([k, x]) => k === "tool" && x.includes("space_tabs")), "a tool call shows", 10000);
await t.until(async () => (await A.rows()).some(([k, x]) => k === "tool" && x.includes("done")), "and finishes", 10000);
await t.until(async () => (await assistantRows()).length === 2, "its reply follows", 15000);

// A failed turn says why
await A.type("/error");
await A.key("Enter");
await t.until(async () => (await A.text(".unanswered")).includes("asked to fail"), "an unanswered turn shows the reason", 10000);
await t.shot("agent-unanswered");

// Escape closes; the conversation is still there next time
await A.key("Escape");
await t.until(async () => !(await A.request()), "escape closes the panel");
await A.open();
await t.until(async () => (await userRows()).length === 3, "reopening shows the same conversation");

// Each space has its own agent
await A.key("Escape");
const second = await t.invoke("create_space");
await t.invoke("activate_space", { id: second });
await t.until(() => S().activeSpace === second, "second space active");
await A.open();
await t.assert((await A.request()).space.id === second, "the panel follows the active space");
await t.until(async () => (await A.rows()).length === 0, "the new space's agent starts empty");
await t.shot("agent-other-space");
await A.key("Escape");
await t.invoke("activate_space", { id: first });
await t.until(() => S().activeSpace === first, "first space active again");
await A.open();
await t.until(async () => (await userRows()).length === 3, "the first space's conversation is back");

// Stop ends a turn
await A.type("one more question");
await A.key("Enter");
await t.until(async () => (await A.text(".presence")).length > 0, "answering");
await t.eval("agent", `document.querySelector("button.stop").click()`);
await t.until(async () => !(await A.text(".presence")), "stop ends the turn", 10000);
await t.assert((await assistantRows()).length === 2, "no reply was added for the stopped turn");

// A new conversation, and the old one in the list
await t.eval("agent", `document.querySelector('button[title="New conversation"]').click()`);
await t.until(async () => (await A.rows()).length === 0, "+ starts a new conversation");
await A.type("second conversation");
await A.key("Enter");
await t.until(async () => (await assistantRows()).length === 1, "it gets its own reply", 15000);
await t.until(async () => (await t.read("agent", `document.querySelectorAll("select.threads option").length`)) === "3", "both conversations are listed (plus New)");

// A crash in the agent process restarts it, not Ternt
await A.type("/crash");
await A.key("Enter");
await t.until(async () => (await t.read("agent", `document.querySelector(".status")?.dataset.state ?? ""`)) !== "", "the panel reports the crash", 10000);
await t.until(async () => (await t.invoke("agent_status")).state === "running", "the process comes back", 15000);
await t.until(async () => (await t.read("agent", `document.querySelector(".status")?.dataset.state ?? ""`)) === "restarted", "and the panel says it was restarted");
await t.shot("agent-restarted");
await A.type("still here?");
await A.key("Enter");
await t.until(async () => (await assistantRows()).some(([, x]) => x.includes("standing in")), "a new question is answered after the restart", 15000);

// One overlay at a time
await t.palette.open("new-tab");
await t.until(async () => !(await A.request()), "opening the command bar closes the panel");
await t.palette.key("Escape");
await A.open();
await t.until(async () => !(await t.palette.request()), "opening the panel closes the command bar");
await A.key("Escape");
await t.quit();

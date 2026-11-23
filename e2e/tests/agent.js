// The Tokidoki panel (⌃A) against the stub agent: per-space conversations,
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

// The command bar reaches the agent too: `tok` ⇥ and what is typed starts a conversation
const threadsBefore = (await t.invoke("agent_threads", { space: S().activeSpace })).length;
await t.palette.open("new-tab");
await t.palette.type("tok");
await t.until(async () => (await t.palette.rows()).includes("Ask Tokidoki"), "typing tok offers Tokidoki among the suggestions");
await t.palette.key("Tab");
await t.until(async () => (await t.read("palette", `document.querySelector(".engine")?.dataset.engine ?? ""`)) === "tokidoki", "Tab makes it the pill");
await t.palette.type("hello from the command bar");
await t.until(async () => (await t.palette.rows()).some((row) => row.includes("Ask Tokidoki “hello from the command bar”")), "what is typed is offered as a question for it");
await t.palette.key("Enter");
await t.until(() => A.request(), "Enter opens the panel");
await t.assert(!(await t.palette.request()), "and closes the command bar");
await t.until(async () => (await userRows()).some(([, x]) => x.includes("hello from the command bar")), "with the question asked", 15000);
await t.until(async () => (await userRows()).length === 1, "in a conversation of its own");
await t.until(async () => (await assistantRows()).length >= 1, "and answered", 15000);
await t.until(async () => (await t.invoke("agent_threads", { space: S().activeSpace })).length === threadsBefore + 1, "a new conversation was started");
await A.key("Escape");
// Picked from the suggestions with nothing to say, it just opens the panel
await t.palette.open("new-tab");
await t.palette.type("tok");
await t.until(async () => (await t.palette.rows()).includes("Ask Tokidoki"), "offered again");
await t.eval("palette", `[...document.querySelectorAll("li")].find((li) => li.textContent.includes("Ask Tokidoki")).dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0 }))`);
await t.until(async () => (await t.read("palette", `document.querySelector(".engine")?.dataset.engine ?? ""`)) === "tokidoki", "clicking the suggestion makes it the pill too");
await t.palette.key("Enter");
await t.until(() => A.request(), "and Enter with nothing typed opens the panel");
await t.sleep(400);
await t.assert((await t.invoke("agent_threads", { space: S().activeSpace })).length === threadsBefore + 1, "without starting another conversation");
await A.key("Escape");

// The switch (Settings → Agent) holds whatever the process was doing when it was flipped
const pause = (ms) => new Promise((done) => setTimeout(done, ms));
const state = async () => (await t.invoke("agent_status")).state;
const prefs = await t.invoke("get_prefs");
const turn = (agentEnabled) => t.invoke("set_prefs", { prefs: { ...prefs, agentEnabled } });
await turn(false);
await t.until(async () => (await state()) === "down", "switched off, the process stops");
await pause(1500);
await t.assert((await state()) === "down", "and stays stopped");
await turn(true);
await turn(false);
await turn(true);
await t.until(async () => (await state()) === "running", "on, off and on again at once leaves it running", 15000);
await pause(1500);
await t.assert((await state()) === "running" && (await t.invoke("get_prefs")).agentEnabled, "as the switch says, with no late stop");
await turn(false);
await turn(true);
await turn(false);
await t.until(async () => (await state()) === "down", "and ending on off leaves it stopped", 15000);
await pause(1500);
await t.assert((await state()) === "down", "with no late start");
await turn(true);
await t.until(async () => (await state()) === "running", "switched on, it runs again", 15000);
await t.quit();

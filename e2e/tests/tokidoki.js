// The panel against the real agent, tokidoki-serve, built from the tokidoki repo:
//
//   TERNT_SIDECAR=tokidoki e2e/run.sh tokidoki
//
// Without an Anthropic key in the environment it checks that the panel asks for one and
// holds input back. With FIREWORKS_API_KEY (or ANTHROPIC_API_KEY) set it also asks one short question
// (one real model call), then restarts the process and checks the conversation survived
// on disk. Against the stub sidecar it has nothing to test and says so.
const S = () => t.state();
const A = t.agent;
const userRows = async () => (await A.rows()).filter(([k]) => k === "user");
const assistantRows = async () => (await A.rows()).filter(([k]) => k === "assistant");
const running = () =>
  t.until(async () => {
    const s = await t.invoke("agent_status");
    return s.state === "running" ? s : null;
  }, "the agent process is running", 20000);

await A.open();
const status = await running();
if (status.backend !== "tokidoki") {
  await t.log(`warn - the sidecar is ${status.backend}, not tokidoki; run with TERNT_SIDECAR=tokidoki`);
  await t.quit();
}
await t.assert(status.restarted === false, "tokidoki is up, on its first start");
await t.until(async () => (await A.text(".badge")) === "", "no stub badge for the real agent");
const first = S().activeSpace;
await t.assert((await A.request()).folder === `space-${first}`, "the space's folder name is one tokidoki accepts");

if (!status.hasKey) {
  await t.until(async () => (await A.text(".keys")).includes("needs an API key"), "without a key the panel asks for one");
  await t.until(async () => (await t.read("agent", `String(document.querySelector("textarea").disabled)`)) === "true", "and holds input back");
  await t.until(async () => (await t.read("agent", `document.querySelector("textarea").placeholder`)).includes("API key"), "the composer says why");
  await t.shot("tokidoki-needs-key");
  // Listing threads needs no model: an empty list, or (older tokidoki, which opened the agent
  // for every method) an error naming the key.
  const listed = await t.invoke("agent_threads", { space: first }).then((v) => v, (e) => String(e));
  await t.assert((Array.isArray(listed) && listed.length === 0) || /API_KEY/.test(listed), `threads without a key: ${JSON.stringify(listed)}`);
  await A.key("Escape");
  await t.quit();
}

await t.until(async () => (await A.text(".keys")) === "", "with a key there is no keys card");
await t.until(async () => (await t.read("agent", `String(document.querySelector("textarea").disabled)`)) === "false", "and input is open");
await A.type("Reply with exactly the word pong and nothing else.");
await A.key("Enter");
await t.until(async () => (await userRows()).length === 1, "the question shows as a user row");
await t.until(async () => (await A.rows()).some(([k]) => k === "live") || (await assistantRows()).length === 1, "text streams in as deltas", 60000);
await t.until(async () => (await assistantRows()).some(([, x]) => /pong/i.test(x)), "the reply lands as a checkpoint", 90000);
await t.until(async () => !(await A.rows()).some(([k]) => k === "live") && !(await A.text(".presence")), "streaming and presence end with the turn", 20000);
await t.shot("tokidoki-reply");

// What tokidoki keeps on disk comes back after the process is restarted on purpose
await t.invoke("agent_restart");
await t.until(async () => (await t.invoke("agent_status")).state !== "running" || (await t.invoke("agent_status")).restarted === false, "the process is going down");
const again = await running();
await t.assert(again.restarted === false, "an asked-for restart isn't reported as a crash");
await t.until(async () => (await t.read("agent", `document.querySelector(".status")?.dataset.state ?? ""`)) === "", "no crash banner");
await t.until(async () => (await userRows()).length === 1 && (await assistantRows()).some(([, x]) => /pong/i.test(x)), "the conversation is back from disk", 15000);
await t.until(async () => (await t.invoke("agent_threads", { space: first })).length === 1, "and listed");
await A.key("Escape");
await t.quit();

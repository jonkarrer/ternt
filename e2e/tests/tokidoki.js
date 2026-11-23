// The panel against the real agent, tokidoki-serve, built from the tokidoki repo, on each
// provider's cheapest model (tokidoki.env sets TOKIDOKI_MODELS=cheap):
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
let space = await t.invoke("agent_space_status", { space: first });
await t.assert(typeof space.model === "string" && space.model.length > 0, `the space's agent says which model answers (${space.model})`);
await t.assert(space.provider === S().spaces.find((s) => s.id === first).agent.provider, "on the provider the space chose");

if (!space.hasKey) {
  await t.until(async () => (await A.text(".keys")).includes("API key"), "without a key for the provider the panel points at Settings");
  // Another provider's key may be around (Doppler has the Anthropic one): switching the
  // space's provider runs its agent on that key, with no restart.
  const have = await t.invoke("agent_keys");
  const other = space.provider === "fireworks" ? "anthropic" : "fireworks";
  if (have[other] !== "none") {
    space = await t.invoke("agent_set_space", { space: first, setup: { provider: other, tools: [] } });
    await t.assert(space.provider === other && space.hasKey, `switching the space to ${other} runs its agent on that key`);
    await t.assert((await t.invoke("agent_status")).generation === status.generation, "with no restart");
  }
}
if (!space.hasKey) {
  await t.until(async () => (await t.read("agent", `String(document.querySelector("textarea").disabled)`)) === "true", "and holds input back");
  await t.until(async () => (await t.read("agent", `document.querySelector("textarea").placeholder`)).includes("Settings"), "the composer says where to go");
  await t.shot("tokidoki-needs-key");
  // Listing threads needs no model: an empty list from an agent opened asleep.
  const listed = await t.invoke("agent_threads", { space: first });
  await t.assert(Array.isArray(listed) && listed.length === 0, "threads without a key: an empty list");
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
const before = (await t.invoke("agent_status")).generation;
await t.invoke("agent_restart");
const again = await t.until(async () => {
  const s = await t.invoke("agent_status");
  return s.state === "running" && s.generation !== before ? s : null;
}, "a new process is running", 20000);
await t.assert(again.restarted === false, "an asked-for restart isn't reported as a crash");
await t.until(async () => (await t.read("agent", `document.querySelector(".status")?.dataset.state ?? ""`)) === "", "no crash banner");
await t.until(async () => (await userRows()).length === 1 && (await assistantRows()).some(([, x]) => /pong/i.test(x)), "the conversation is back from disk", 15000);
await t.until(async () => (await t.invoke("agent_threads", { space: first })).length === 1, "and listed");

// A space switches provider without a restart and keeps its conversation (no model call:
// the agent is stopped and opened again under the other provider, asleep if there's no key).
const have = await t.invoke("agent_keys");
const other = space.provider === "fireworks" ? "anthropic" : "fireworks";
const switched = await t.invoke("agent_set_space", { space: first, setup: { provider: other, tools: [] } });
await t.assert(switched.provider === other && switched.hasKey === (have[other] !== "none"), `the space now runs on ${other} (key: ${switched.hasKey})`);
await t.assert((await t.invoke("agent_status")).generation === again.generation, "with no restart of the process");
await t.until(async () => (await t.invoke("agent_threads", { space: first })).length === 1, "and the conversation is still there");
await t.until(async () => (await t.read("agent", `document.querySelector(".keys")?.textContent ?? ""`)).includes("API key") === !switched.hasKey, "the panel follows: a keys card only if the new provider has no key");
await t.invoke("agent_set_space", { space: first, setup: { provider: space.provider, tools: [] } });
await t.until(async () => (await t.invoke("agent_space_status", { space: first })).provider === space.provider, "and back");

// Memory is read and wiped without a model call (what it learned from one "pong" is its business)
const remembered = await t.invoke("agent_memories", { space: first });
await t.assert(Array.isArray(remembered) && remembered.every((m) => m.kind === "fact" || m.kind === "procedure"), `it lists what it remembers (${remembered.length})`);
await t.invoke("agent_wipe", { space: first });
await t.assert((await t.invoke("agent_memories", { space: first })).length === 0, "wipe leaves nothing");
await t.assert((await t.invoke("agent_threads", { space: first })).length === 1, "but keeps the conversation");
await A.key("Escape");
await t.quit();

// Runs in the installed app at each version (see e2e/update.sh): 0.1.0
// sees 0.1.1 download in the background (then gets killed, and its next
// launch installs it), 0.1.1 takes the required 0.1.2 with no click, and
// 0.1.2 installs 0.1.3 when it quits.
const version = await t.invoke("app_version");
const card = () => document.querySelector(".update-card");
const press = (label) => [...card().querySelectorAll("button")].find((b) => b.textContent.includes(label)).click();
await t.log(`running version ${version}`);

if (version === "0.1.0") {
  // The second launch installs the kept download and relaunches before this
  // gets far; the first sees the card.
  await t.until(() => card()?.textContent.includes("Ternt 0.1.1 is ready"), "the update downloads by itself and a card says so", 60000);
  await t.assert(card().textContent.includes("installs when you quit"), "it installs on quit");
  press("Later");
  await t.until(() => !card(), "Later hides the card");
  await t.invoke("check_for_updates");
  await t.sleep(1500);
  await t.assert(!card(), "and keeps it hidden for that version");
  await t.log("READY 0.1.1");
  await t.shot("update-later");
  await t.sleep(600000); // killed by the script
} else if (version === "0.1.1") {
  // 0.1.2 is required: no buttons, it updates and relaunches by itself.
  await t.until(() => /Updating to 0.1.2|Restarting/.test(card()?.textContent ?? ""), "a required update installs with no click", 60000);
  await t.assert(!card().querySelector("button"), "no buttons to press");
  await t.log("waiting for the relaunch");
  await t.sleep(600000);
} else if (version === "0.1.2") {
  await t.until(() => card()?.textContent.includes("Ternt 0.1.3 is ready"), "0.1.3 downloads by itself", 60000);
  await t.log("QUITTING");
  await t.quit(); // installs 0.1.3 (the script checks the bundle)
} else {
  await t.assert(false, `unexpected version ${version}`);
}

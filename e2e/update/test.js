// Runs in the installed app twice: first as 0.1.0 (finds the update, clicks
// Restart Now on the card), then, after the app swaps itself and relaunches,
// as the new version. See e2e/update.sh.
const version = await t.invoke("app_version");
const card = () => document.querySelector(".update-card");
const press = (label) => [...card().querySelectorAll("button")].find((b) => b.textContent.includes(label)).click();
await t.log(`running version ${version}`);

if (version === "0.1.0") {
  await t.until(() => card()?.textContent.includes("Ternt 0.1.1 is ready"), "a card pops up in the sidebar", 30000);
  await t.assert(!document.querySelector("button.update"), "no update pill");
  await t.sleep(250);
  await t.shot("update-available");
  press("Later");
  await t.until(() => !card(), "Later hides the card");
  await t.invoke("check_for_updates");
  await t.sleep(1500);
  await t.assert(!card(), "and keeps it hidden for that version");
  // Settings → General still offers it; installing shows progress in the card.
  t.invoke("install_update").catch(() => {}); // resolves only if it fails: success relaunches
  await t.until(() => /Updating|Restarting/.test(card()?.textContent ?? ""), "installing shows progress on the card");
  await t.log("waiting for the relaunch");
} else {
  await t.assert(version === "0.1.1", `relaunched as the new version (${version})`);
  // The background check runs again and finds nothing newer.
  await t.sleep(8000);
  await t.assert(!card(), "no update offered once current");
  await t.invoke("check_for_updates");
  await t.until(() => card()?.textContent.includes("up to date"), "a manual check says up to date");
  await t.shot("update-done");
  await t.log("UPDATE TEST PASSED");
  await t.quit();
}

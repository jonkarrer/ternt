// Runs in the installed app twice: first as 0.1.0 (finds the update, clicks
// the button), then, after the app swaps itself and relaunches, as the new
// version. See e2e/update.sh.
const version = await t.invoke("app_version");
const button = () => document.querySelector(".update");
await t.log(`running version ${version}`);

if (version === "0.1.0") {
  await t.until(() => button()?.textContent.includes("Update"), "the update button appears (bottom left)", 30000);
  await t.assert(button().title.includes("0.1.1"), "it names the new version");
  await t.shot("update-available");
  button().click();
  await t.until(() => /Updating|Restarting/.test(button()?.textContent ?? ""), "clicking starts the download");
  await t.log("waiting for the relaunch");
} else {
  await t.assert(version === "0.1.1", `relaunched as the new version (${version})`);
  // The background check runs again and finds nothing newer.
  await t.sleep(8000);
  await t.assert(!button(), "no update offered once current");
  await t.invoke("check_for_updates");
  await t.until(() => button()?.textContent.includes("Up to date"), "a manual check says up to date");
  await t.shot("update-done");
  await t.log("UPDATE TEST PASSED");
  await t.quit();
}

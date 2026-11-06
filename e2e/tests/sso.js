// Sign-in popups: the account page is on another origin, its callback
// posts back to the app and closes the popup.
const S = () => t.state();
const tab = () => S().tabs.find((x) => x.id === S().active);
const F = "http://localhost:8765";
const app = await t.invoke("new_tab", { url: `${F}/sso.html` });
await t.until(() => tab()?.title === "SSO App" && !tab().loading, "the app loads");
await t.invoke("debug_allow_popups", { label: `tab-${app}` });
await t.eval(`tab-${app}`, "signIn()");
let popup;
await t.until(async () => (popup = (await t.invoke("debug_windows")).find((w) => w.startsWith("popup-"))), "a sign-in window opens");
await t.sleep(800);
await t.invoke("debug_eval", { label: popup, js: `document.getElementById("account").click()` });
await t.until(() => tab()?.title.startsWith("signed in"), `the app hears back (${tab()?.title})`, 5000);
await t.until(() => tab()?.title === "signed in, popup closed", `and the popup closes (${tab()?.title})`, 5000);
await t.until(async () => !(await t.invoke("debug_windows")).some((w) => w.startsWith("popup-")), "its window is gone");

// A tab a page opened can close itself too; one you opened can't
const tabs = S().tabs.length;
await t.eval(`tab-${app}`, `window.open("/closes.html", "_blank")`);
await t.until(() => S().tabs.length === tabs + 1, "a page opens a tab");
await t.until(() => S().tabs.length === tabs && S().active === app, "which closes itself, back to the page");
await t.invoke("navigate", { url: `${F}/closes.html` });
await t.sleep(1000);
await t.assert(S().tabs.length === tabs && S().tabs.some((x) => x.id === app), "a tab you opened stays");
await t.quit();

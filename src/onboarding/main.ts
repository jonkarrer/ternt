import { mount } from "svelte";
import Onboarding from "./Onboarding.svelte";
import "../theme.css";
// The setup window is Settings' sibling: the same cards, rows and controls.
import "../settings/settings.css";

export default mount(Onboarding, { target: document.getElementById("app")! });

import { mount } from "svelte";
import Settings from "./Settings.svelte";
import "../theme.css";
import "./settings.css";

export default mount(Settings, { target: document.getElementById("app")! });

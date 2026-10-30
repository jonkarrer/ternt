import { mount } from "svelte";
import Little from "./Little.svelte";
import "../theme.css";

export default mount(Little, { target: document.getElementById("app")! });

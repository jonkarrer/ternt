import { mount } from "svelte";
import Palette from "./Palette.svelte";
import "../theme.css";

export default mount(Palette, { target: document.getElementById("app")! });

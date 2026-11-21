import { mount } from "svelte";
import Agent from "./Agent.svelte";
import "../theme.css";

export default mount(Agent, { target: document.getElementById("app")! });

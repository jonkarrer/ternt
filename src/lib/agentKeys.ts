// The API keys the agent runs on, as Settings → Agent and the setup window list them. They
// are the user's own, shared between spaces: kept in the Keychain, handed to the agent
// process, nowhere else.
import type { AgentKeys, AgentProvider } from "./browser";

export interface KeyInfo {
  id: keyof AgentKeys;
  name: string;
  /** The field's placeholder while no key is set. */
  keyHint: string;
  /** Where to make one. */
  where: string;
  /** What it is for, when its name doesn't say. */
  what?: string;
  /** Not needed to answer, but the agent is much better with it. */
  recommended?: boolean;
}

/** Who can answer: a space picks one, and the agent needs that one's key. */
export const PROVIDERS: (KeyInfo & { id: AgentProvider })[] = [
  { id: "fireworks", name: "Fireworks AI", keyHint: "fw_…", where: "fireworks.ai → API keys" },
  { id: "anthropic", name: "Anthropic", keyHint: "sk-ant-…", where: "console.anthropic.com → API keys" },
];

/** Jev, from TypeSafe. Not a provider and not a tool: it makes the small decisions in every space. */
export const JEV: KeyInfo = {
  id: "typesafe",
  name: "Jev",
  keyHint: "Recommended…",
  where: "console.typesafe.ai/keys",
  what: "Decides where a message goes and what to recall or remember, quickly and for very little.",
  recommended: true,
};

export const EXA: KeyInfo = {
  id: "exa",
  name: "Exa",
  keyHint: "Web search…",
  where: "dashboard.exa.ai → API keys",
  what: "Lets a space's agent search the web, where you switch that on.",
};

/** Every key, in the order they matter: who answers, Jev, then the tools'. */
export const KEYS: KeyInfo[] = [...PROVIDERS, JEV, EXA];
